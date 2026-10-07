//! `LogService` tests over `FakeLogPort` scripts, on a deterministic executor (no runtime, no
//! threads) and the fake clock.

mod batching;
mod cancel;
mod clear;
mod delta;
mod errors;
mod export;
mod filter;
mod filter_index;
mod filter_props;
mod hot_reload;
mod options;
mod props;
mod ring;
mod structured;

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::future::BoxFuture;
use futures::task::LocalSpawnExt;
use jiff::Timestamp;
use oxikube_domain::log::LogLine;
use oxikube_ports::LogOptions;
use oxikube_testkit::{FakeClockPort, FakeLogPort, Timeline};
use parking_lot::Mutex;

use super::{LogConfig, LogRuntime, LogService, LogSession, LogTarget};

/// Runs spawned tasks on a `LocalPool`, only when the test says so.
#[derive(Clone, Default)]
pub(super) struct Executor {
    queue: Arc<Mutex<Vec<BoxFuture<'static, ()>>>>,
}

impl Executor {
    pub fn spawner(&self) -> Arc<dyn crate::store::Spawner> {
        let queue = self.queue.clone();
        Arc::new(move |task: BoxFuture<'static, ()>| queue.lock().push(task))
    }

    /// Polls every task until none can make progress.
    pub fn run(&self, pool: &mut LocalPool) {
        loop {
            let tasks = std::mem::take(&mut *self.queue.lock());
            for task in tasks {
                pool.spawner().spawn_local(task).expect("spawn");
            }
            pool.run_until_stalled();
            if self.queue.lock().is_empty() {
                return;
            }
        }
    }
}

/// The server timestamp of line `i`: one millisecond apart.
pub(super) fn ts(i: usize) -> Timestamp {
    Timestamp::from_millisecond(1_760_000_000_000 + i as i64).unwrap()
}

/// Line `i` of pod `web-0`'s container `app`.
pub(super) fn line(i: usize) -> LogLine {
    LogLine::new(ts(i), "web-0", "app", format!("line {i}"))
}

/// A service over a `FakeLogPort` plus the controls a test needs.
pub(super) struct Harness {
    pub clock: Arc<FakeClockPort>,
    pub port: Arc<FakeLogPort>,
    pub service: LogService,
    exec: Executor,
    pool: LocalPool,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_config(LogConfig::default())
    }

    pub fn with_config(config: LogConfig) -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let port = Arc::new(FakeLogPort::with_clock(clock.clone()));
        let exec = Executor::default();
        let service = LogService::new(
            LogRuntime {
                spawner: exec.spawner(),
                clock: clock.clone(),
            },
            config,
        );
        Self {
            clock,
            port,
            service,
            exec,
            pool: LocalPool::new(),
        }
    }

    /// Queues `timeline` as the next stream and opens `target` with `options`, settled.
    pub fn open(
        &mut self,
        timeline: Timeline<LogLine>,
        target: LogTarget,
        options: LogOptions,
    ) -> LogSession {
        self.port.script().stream_logs.push_ok(timeline);
        let session = self.service.open(self.port.clone(), target, options);
        self.settle();
        session
    }

    /// Opens `web-0` with follow options over `timeline`, settled.
    pub fn follow(&mut self, timeline: Timeline<LogLine>) -> LogSession {
        self.open(
            timeline,
            LogTarget::pod("default", "web-0"),
            LogOptions::follow(),
        )
    }

    /// Like [`follow`](Self::follow), then lets the flush tick fire so the last partial batch is
    /// committed.
    pub fn follow_flushed(&mut self, timeline: Timeline<LogLine>) -> LogSession {
        let session = self.follow(timeline);
        self.advance(LogConfig::default().flush_interval);
        session
    }

    /// Runs every task until idle.
    pub fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }

    /// Advances the fake clock by `by` and settles.
    pub fn advance(&mut self, by: Duration) {
        self.clock.advance(by);
        self.settle();
    }
}

/// A timeline of `n` lines, all at once.
pub(super) fn burst(n: usize) -> Timeline<LogLine> {
    Timeline::immediate((0..n).map(line))
}
