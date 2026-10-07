//! The harness the excerpt, `@logs` and `get_logs` tests share: a `LogService` over fake ports on
//! a deterministic executor and the fake clock, and a way to drive a future that waits on them.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::executor::LocalPool;
use futures::task::noop_waker_ref;
use jiff::Timestamp;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::log::LogLine;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_testkit::{FakeClockPort, FakeLogPort, FakeResourcePort, Timeline, pod};

use super::{LogCluster, LogClusters};
use crate::logs::tests::Executor;
use crate::logs::{AggregatePorts, LogConfig, LogRuntime, LogService};

pub(crate) fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The server timestamp `at` milliseconds into the fixture's hour.
pub(crate) fn ts(at: i64) -> Timestamp {
    Timestamp::from_millisecond(1_760_000_000_000 + at).unwrap()
}

/// A line of `pod`'s container `app`, stamped `at`.
pub(crate) fn line(pod: &str, at: i64, text: &str) -> LogLine {
    LogLine::new(ts(at), pod, "app", text)
}

/// A running pod `name` labelled `app=web` (container `app`).
pub(crate) fn web_pod(name: &str) -> Resource {
    pod()
        .name(name)
        .namespace("default")
        .uid(format!("uid-{name}"))
        .label("app", "web")
        .build()
}

/// The cluster the fakes stand for.
pub(crate) fn cluster_id() -> ClusterId {
    ClusterId::new("~/.kube/config", &ContextName::new("kind"))
}

/// A [`LogClusters`] that knows one cluster (or none).
pub(crate) struct FixedCluster(pub Option<LogCluster>);

impl LogClusters for FixedCluster {
    fn cluster(&self, cluster: Option<&ClusterId>) -> OxiResult<LogCluster> {
        match (&self.0, cluster) {
            (Some(known), Some(asked)) if &known.id != asked => Err(OxiError::not_found(format!(
                "no session for cluster {asked}"
            ))),
            (Some(known), _) => Ok(known.clone()),
            (None, _) => Err(OxiError::network("no cluster is connected")),
        }
    }
}

pub(crate) struct Env {
    pub clock: Arc<FakeClockPort>,
    pub logs: Arc<FakeLogPort>,
    pub resources: Arc<FakeResourcePort>,
    pub service: Arc<LogService>,
    exec: Executor,
    pool: LocalPool,
}

impl Env {
    pub fn new() -> Self {
        let clock = Arc::new(FakeClockPort::default());
        let exec = Executor::default();
        let service = Arc::new(LogService::new(
            LogRuntime {
                spawner: exec.spawner(),
                clock: clock.clone(),
            },
            LogConfig::default(),
        ));
        Self {
            logs: Arc::new(FakeLogPort::with_clock(clock.clone())),
            resources: Arc::new(FakeResourcePort::with_clock(clock.clone())),
            clock,
            service,
            exec,
            pool: LocalPool::new(),
        }
    }

    pub fn ports(&self) -> AggregatePorts {
        AggregatePorts {
            logs: self.logs.clone(),
            resources: self.resources.clone(),
        }
    }

    /// The cluster these fakes stand for, as a [`LogClusters`].
    pub fn clusters(&self) -> Arc<FixedCluster> {
        Arc::new(FixedCluster(Some(LogCluster {
            id: cluster_id(),
            title: "kind".into(),
            ports: self.ports(),
        })))
    }

    /// Queues `timeline` as the next stream.
    pub fn script(&self, timeline: Timeline<LogLine>) {
        self.logs.script().stream_logs.push_ok(timeline);
    }

    /// `n` lines of `web-0`, one millisecond apart.
    pub fn lines(&self, n: i64) -> Timeline<LogLine> {
        Timeline::immediate((0..n).map(|i| line("web-0", i, &format!("line {i}"))))
    }

    /// Drives `future` to its end: runs the session tasks and lets fake time pass in 50 ms steps.
    pub fn run<T>(&mut self, future: impl Future<Output = T>) -> T {
        let mut future = Box::pin(future);
        let mut cx = Context::from_waker(noop_waker_ref());
        for _ in 0..800 {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            self.exec.run(&mut self.pool);
            self.clock.advance(ms(50));
            self.exec.run(&mut self.pool);
        }
        panic!("the future never finished");
    }

    /// Runs the tasks that are ready (a dropped session's cancellation).
    pub fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }
}
