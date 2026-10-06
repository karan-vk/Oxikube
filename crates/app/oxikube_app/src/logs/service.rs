//! [`LogService`]: opens log sessions over a `LogPort` and keeps their memory bounded.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use oxikube_ports::{LogOptions, LogPort};
use parking_lot::Mutex;
use std::sync::Weak;

use super::driver::Driver;
use super::options::{LogConfig, LogRuntime, clamp_buffer_lines};
use super::session::{LogReader, LogSession};
use super::shared::Shared;
use super::target::LogTarget;
use crate::store::spawn_guarded;

/// Opens and tracks log sessions. One per app: the sessions of every cluster share the
/// `logs.buffer_lines` bound and the runtime.
///
/// Plain async Rust over [`LogPort`]: no gpui, no kube. The UI, the MCP `get_logs` tool and the
/// tests share this one implementation.
pub struct LogService {
    runtime: LogRuntime,
    config: LogConfig,
    buffer_lines: Arc<AtomicUsize>,
    next_id: AtomicU64,
    sessions: Mutex<Vec<Weak<Shared>>>,
}

impl LogService {
    /// A service that runs session tasks on `runtime.spawner`. Nothing is spawned until a session
    /// is opened.
    pub fn new(runtime: LogRuntime, config: LogConfig) -> Self {
        Self {
            runtime,
            buffer_lines: Arc::new(AtomicUsize::new(clamp_buffer_lines(config.buffer_lines))),
            config,
            next_id: AtomicU64::new(1),
            sessions: Mutex::new(Vec::new()),
        }
    }

    /// Opens a session reading `target` over `port` with `options`, and returns at once: the
    /// stream is opened on the runtime, never on the caller. The session starts `Connecting`;
    /// a pod that does not exist or a denied `pods/log` is its `Failed` state, not an error here.
    ///
    /// The container is the target's, else `options.container`; the session's
    /// [`target`](LogReader::target) and [`options`](LogReader::options) say which won.
    pub fn open(
        &self,
        port: Arc<dyn LogPort>,
        mut target: LogTarget,
        mut options: LogOptions,
    ) -> LogSession {
        let container = target.container.take().or(options.container.take());
        target.container.clone_from(&container);
        options.container = container;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let capacity = self.buffer_lines();
        let shared = Arc::new(Shared::new(id, target, options, capacity));
        {
            let mut sessions = self.sessions.lock();
            sessions.retain(|s| s.strong_count() > 0);
            sessions.push(Arc::downgrade(&shared));
        }
        let driver = Driver {
            port,
            shared: shared.clone(),
            clock: self.runtime.clock.clone(),
            config: self.config.clone(),
            buffer_lines: self.buffer_lines.clone(),
        };
        let task = spawn_guarded(&self.runtime.spawner, driver.run());
        LogSession::new(shared, task)
    }

    /// Lines each session keeps (`logs.buffer_lines`).
    pub fn buffer_lines(&self) -> usize {
        self.buffer_lines.load(Ordering::Relaxed)
    }

    /// Applies a new `logs.buffer_lines` (clamped) to every open session at once and to the
    /// ones opened later. A smaller bound drops the oldest lines of the sessions now.
    pub fn set_buffer_lines(&self, lines: usize) {
        let lines = clamp_buffer_lines(lines);
        self.buffer_lines.store(lines, Ordering::Relaxed);
        for shared in self.live() {
            shared.set_capacity(lines);
        }
    }

    /// Read-only views of the open sessions, oldest first.
    pub fn sessions(&self) -> Vec<LogReader> {
        self.live().into_iter().map(LogReader::new).collect()
    }

    /// The open session reading `target`, if any (the newest, when several are open).
    pub fn reader_for(&self, target: &LogTarget) -> Option<LogReader> {
        self.live()
            .into_iter()
            .rev()
            .find(|s| &s.target == target)
            .map(LogReader::new)
    }

    fn live(&self) -> Vec<Arc<Shared>> {
        let mut sessions = self.sessions.lock();
        sessions.retain(|s| s.strong_count() > 0);
        sessions
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|shared| !shared.is_closed())
            .collect()
    }
}

impl std::fmt::Debug for LogService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogService")
            .field("buffer_lines", &self.buffer_lines())
            .finish_non_exhaustive()
    }
}
