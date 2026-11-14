//! The wasmtime side of the extension host: one shared [`Engine`], one [`Store`] per extension
//! with a scrubbed WASI context, and an epoch ticker that interrupts guests that run too long.
//!
//! Every guest entry ([`WasmExtension::call`], instantiation) may run for [`CALL_TIMEOUT`]. At
//! each epoch tick the guest yields to the async executor, so the caller's thread is never held
//! for longer than a tick, and the first tick after the timeout traps the call with
//! [`WasmError::TimedOut`]. Calls are async and must run off the UI thread: wrap
//! them in `oxikube_runtime::spawn_kube`, whose task aborts the call when it is dropped (dropping
//! the call future stops the guest at its next tick). Calls into one extension are serialised by
//! a mutex around its store. An interrupted instance (timed out or dropped mid-call) refuses
//! later calls with [`WasmError::Wasm`]; the extension has to be loaded again.

mod log_stream;

use crate::wasm_host::log_stream::LogStream;
use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::Mutex;
use wasmtime::Config;
use wasmtime::Engine;
use wasmtime::Store;
use wasmtime::StoreLimits;
use wasmtime::StoreLimitsBuilder;
use wasmtime::UpdateDeadline;
use wasmtime::component::Component;
use wasmtime::component::ComponentNamedList;
use wasmtime::component::Instance;
use wasmtime::component::Lift;
use wasmtime::component::Linker;
use wasmtime::component::Lower;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::FsPerms;
use wasmtime_wasi::WasiCtx;
use wasmtime_wasi::WasiCtxView;
use wasmtime_wasi::WasiView;

/// How often the ticker advances the engine's epoch.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// How long one guest entry may run before it is interrupted.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(1);

/// Most linear memory one extension may hold (64 MiB).
pub const MAX_MEMORY_BYTES: usize = 64 << 20;

/// Most core instances, tables and memories one extension may create. A Rust `wasm32-wasip2`
/// component uses a handful of each.
const MAX_INSTANCES: usize = 32;

/// Where the extension's working directory appears inside the guest.
const GUEST_WORK_DIR: &str = ".";

/// Why loading or calling an extension failed.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    /// The call ran longer than [`CALL_TIMEOUT`] and was interrupted.
    #[error("extension call timed out")]
    TimedOut,
    /// The working directory could not be created.
    #[error("extension working directory: {0}")]
    WorkDir(#[from] std::io::Error),
    /// wasmtime failed: invalid component, missing export, trap or refused limit.
    #[error("{0:#}")]
    Wasm(wasmtime::Error),
}

impl From<wasmtime::Error> for WasmError {
    fn from(err: wasmtime::Error) -> Self {
        WasmError::Wasm(err)
    }
}

/// The engine every extension shares: component model, async calls and epoch interruption.
///
/// The first call starts the epoch ticker, a thread that advances the epoch every
/// [`EPOCH_TICK`] for the rest of the process. Nothing calls this before the first extension
/// loads, so an app without extensions never starts the ticker.
///
/// # Panics
///
/// If the OS refuses to start the ticker thread.
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.wasm_component_model(true).epoch_interruption(true);
        let engine = Engine::new(&config).expect("the wasm engine configuration is valid");
        let ticking = engine.clone();
        thread::Builder::new()
            .name("wasm-epoch".into())
            .spawn(move || {
                loop {
                    thread::sleep(EPOCH_TICK);
                    ticking.increment_epoch();
                }
            })
            .expect(
                "the wasm epoch ticker thread must start, or guests could never be interrupted",
            );
        engine
    })
}

/// What one extension's [`Store`] holds: its WASI context, resource table and limits, and when
/// the guest entry in progress times out.
pub struct WasmState {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    call_deadline: Instant,
    timed_out: bool,
}

impl WasiView for WasmState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// A loaded extension component: its instance and the store it lives in.
pub struct WasmExtension {
    store: Mutex<Store<WasmState>>,
    instance: Instance,
}

impl WasmExtension {
    /// Compiles and instantiates `component` (a component's binary) for the extension `id`.
    ///
    /// The guest sees only `work_dir` (created if missing), as its current directory: no other
    /// files, no environment variables, no arguments, no stdin and no network. Its stdout and
    /// stderr go to `tracing`, redacted. Compiling is CPU-heavy: call this off the UI thread.
    ///
    /// # Errors
    ///
    /// [`WasmError::WorkDir`] when `work_dir` cannot be created, [`WasmError::Wasm`] when the
    /// component is invalid or imports something the host does not provide, and
    /// [`WasmError::TimedOut`] when instantiation runs too long.
    pub async fn load(
        engine: &Engine,
        id: &str,
        component: &[u8],
        work_dir: &Path,
    ) -> Result<Self, WasmError> {
        tokio::fs::create_dir_all(work_dir).await?;
        let wasi = WasiCtx::builder()
            .preopened_dir(work_dir, GUEST_WORK_DIR, FsPerms::ReadWrite)?
            .stdout(LogStream::new(id, "stdout"))
            .stderr(LogStream::new(id, "stderr"))
            .allow_tcp(false)
            .allow_udp(false)
            .build();
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY_BYTES)
            .instances(MAX_INSTANCES)
            .tables(MAX_INSTANCES)
            .memories(MAX_INSTANCES)
            .build();
        let state = WasmState {
            wasi,
            table: ResourceTable::new(),
            limits,
            call_deadline: Instant::now(),
            timed_out: false,
        };
        let mut store = Store::new(engine, state);
        store.limiter(|state| &mut state.limits);
        store.epoch_deadline_callback(|mut store| {
            let state = store.data_mut();
            if Instant::now() >= state.call_deadline {
                state.timed_out = true;
                return Err(wasmtime::format_err!("extension call timed out"));
            }
            Ok(UpdateDeadline::Yield(1))
        });
        start_budget(&mut store);

        let component = Component::new(engine, component)?;
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        let instance = linker
            .instantiate_async(&mut store, &component)
            .await
            .map_err(|err| call_error(&store, err))?;
        Ok(Self {
            store: Mutex::new(store),
            instance,
        })
    }

    /// Calls the export `name` with `params`, waiting for any call already running in this
    /// extension to finish first.
    ///
    /// # Errors
    ///
    /// [`WasmError::TimedOut`] when the guest runs longer than [`CALL_TIMEOUT`],
    /// [`WasmError::Wasm`] when the export is missing, has another signature, or traps.
    pub async fn call<P, R>(&self, name: &str, params: P) -> Result<R, WasmError>
    where
        P: ComponentNamedList + Lower + Send + Sync,
        R: ComponentNamedList + Lift + Send + Sync + 'static,
    {
        let mut store = self.store.lock().await;
        let func = self.instance.get_typed_func::<P, R>(&mut *store, name)?;
        start_budget(&mut store);
        let result = func.call_async(&mut *store, params).await;
        result.map_err(|err| call_error(&store, err))
    }
}

/// Starts the [`CALL_TIMEOUT`] clock for the next guest entry, which yields at every tick.
fn start_budget(store: &mut Store<WasmState>) {
    let state = store.data_mut();
    state.call_deadline = Instant::now() + CALL_TIMEOUT;
    state.timed_out = false;
    store.set_epoch_deadline(1);
}

/// Maps a failed guest entry to [`WasmError::TimedOut`] when it ran out of time.
fn call_error(store: &Store<WasmState>, err: wasmtime::Error) -> WasmError {
    if store.data().timed_out {
        WasmError::TimedOut
    } else {
        WasmError::Wasm(err)
    }
}

#[cfg(test)]
mod tests;
