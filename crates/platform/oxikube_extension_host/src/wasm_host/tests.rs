use super::*;
use futures::FutureExt as _;
use std::pin::pin;
use std::sync::Arc;

/// The test guest built from `fixtures/guest` (see its `Cargo.toml` for how to rebuild it).
const GUEST: &[u8] = include_bytes!("../../fixtures/guest.wasm");

/// Loads the test guest with a fresh working directory.
async fn load() -> (WasmExtension, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let work_dir = dir.path().join("work");
    let extension = WasmExtension::load(engine(), "fixture", GUEST, &work_dir)
        .await
        .expect("the fixture loads");
    (extension, dir)
}

#[tokio::test]
async fn calls_an_export_and_gets_its_value() {
    let (extension, _dir) = load().await;
    let (answer,) = extension.call::<(), (u32,)>("answer", ()).await.unwrap();
    assert_eq!(answer, 42);
}

#[tokio::test]
async fn the_engine_is_shared() {
    assert!(Engine::same(engine(), engine()));
}

/// A spinning guest yields to its executor at every tick and is interrupted at the
/// [`CALL_TICKS`]-th one. The tick count, not wall-clock time, decides the outcome, so the
/// assertions hold however slowly the ticker runs.
#[tokio::test]
async fn a_spinning_guest_times_out_without_blocking_its_executor() {
    let (extension, _dir) = load().await;
    let mut call = pin!(extension.call::<(), ()>("spin", ()));
    let mut yields = 0_u32;
    let result = loop {
        match call.as_mut().now_or_never() {
            Some(result) => break result,
            None => yields += 1,
        }
    };
    assert!(matches!(result, Err(WasmError::TimedOut)), "{result:?}");
    // Every tick but the last handed the thread back to the caller.
    assert_eq!(yields, CALL_TICKS - 1);
}

#[tokio::test]
async fn dropping_the_call_stops_the_guest_and_frees_the_extension() {
    let (extension, _dir) = load().await;
    {
        let mut call = pin!(extension.call::<(), ()>("spin", ()));
        // Runs the guest until the next tick makes it yield.
        assert!(call.as_mut().now_or_never().is_none());
    }
    // The dropped call released the store and the guest is gone: the next call returns at once,
    // refused because an interrupted instance cannot be entered again.
    let next = extension
        .call::<(), (u32,)>("answer", ())
        .now_or_never()
        .expect("the extension is free");
    assert!(matches!(next, Err(WasmError::Wasm(_))), "{next:?}");
}

#[tokio::test]
async fn the_guest_cannot_read_host_files() {
    let (extension, _dir) = load().await;
    let (result,) = extension
        .call::<(String,), (Result<String, String>,)>("read-file", ("/etc/passwd".into(),))
        .await
        .unwrap();
    assert!(result.is_err(), "{result:?}");
}

#[tokio::test]
async fn the_guest_sees_no_environment() {
    assert!(std::env::var_os("PATH").is_some(), "the host has PATH");
    let (extension, _dir) = load().await;
    let (value,) = extension
        .call::<(String,), (Option<String>,)>("env-var", ("PATH".into(),))
        .await
        .unwrap();
    assert_eq!(value, None);
}

#[tokio::test]
async fn the_guest_works_in_its_working_dir() {
    let (extension, dir) = load().await;
    let (written,) = extension
        .call::<(String, String), (Result<(), String>,)>(
            "write-file",
            ("note.txt".into(), "hello".into()),
        )
        .await
        .unwrap();
    assert_eq!(written, Ok(()));
    let on_host = std::fs::read_to_string(dir.path().join("work/note.txt")).unwrap();
    assert_eq!(on_host, "hello");
}

#[tokio::test]
async fn guest_stdout_goes_to_tracing_redacted() {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (extension, _dir) = load().await;
    extension
        .call::<(String,), ()>("print", ("Authorization: Bearer s3cr3t-token".into(),))
        .await
        .unwrap();

    let logs = logs.text();
    assert!(logs.contains("extension=fixture"), "{logs}");
    assert!(logs.contains("stream=\"stdout\""), "{logs}");
    assert!(logs.contains("Authorization"), "{logs}");
    assert!(!logs.contains("s3cr3t-token"), "{logs}");
}

/// A `tracing` writer that keeps everything written to it.
#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Logs;

    fn make_writer(&'a self) -> Logs {
        self.clone()
    }
}

/// Stub for E23-S11, which asserts every limit.
#[tokio::test]
async fn memory_growth_stops_at_the_limit() {
    let (extension, _dir) = load().await;
    let (small,) = extension
        .call::<(u32,), (bool,)>("grow-memory", (16,))
        .await
        .unwrap();
    assert!(small);
    let pages = u32::try_from(MAX_MEMORY_BYTES / 65536).unwrap();
    let (huge,) = extension
        .call::<(u32,), (bool,)>("grow-memory", (pages,))
        .await
        .unwrap();
    assert!(!huge);
}
