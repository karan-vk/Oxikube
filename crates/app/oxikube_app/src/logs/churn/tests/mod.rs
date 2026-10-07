//! Reconnect and churn tests over `FakeLogPort` and `FakeResourcePort` scripts, on the
//! deterministic executor and the fake clock (no runtime, no threads, no sleeps).

mod fate;
mod reconnect;
mod replacement;

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::log::LogLine;
use oxikube_ports::LogOptions;
use oxikube_testkit::{FakeClockPort, FakeLogPort, FakeResourcePort, LogCall, Timeline};
use serde_json::{Value, json};

use crate::logs::tests::Executor;
use crate::logs::{AggregatePorts, LogConfig, LogRuntime, LogService, LogSession, LogTarget};

/// The server timestamp `ms` milliseconds into the fixture's hour.
pub(super) fn ts(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(1_760_000_000_000 + ms).unwrap()
}

/// Line `i` of `web-0`, stamped `i` seconds in.
pub(super) fn line(i: i64) -> LogLine {
    LogLine::new(ts(i * 1_000), "web-0", "app", format!("line {i}"))
}

/// Lines `range` at once.
pub(super) fn lines(range: std::ops::Range<i64>) -> Timeline<LogLine> {
    Timeline::immediate(range.map(line))
}

/// A pod `name` (uid `uid`) owned by the controller `kind/owner`, on `node`, with `phase`.
pub(super) fn owned_pod(name: &str, uid: &str, owner: Option<(&str, &str)>, phase: &str) -> Value {
    let mut pod = oxikube_testkit::pod()
        .name(name)
        .namespace("default")
        .uid(uid)
        .label("app", "web")
        .json();
    if let Some((kind, owner)) = owner {
        let api = if kind == "Job" { "batch/v1" } else { "apps/v1" };
        pod["metadata"]["ownerReferences"] = json!([{
            "apiVersion": api, "kind": kind, "name": owner,
            "uid": format!("uid-{owner}"), "controller": true
        }]);
    }
    pod["status"]["phase"] = json!(phase);
    pod
}

pub(super) fn resource(json: Value) -> Resource {
    Resource::from_json(json).expect("a valid object")
}

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("~/.kube/config", &ContextName::new("kind"))
}

/// A service over a fake cluster plus the controls a test needs.
pub(super) struct Harness {
    pub clock: Arc<FakeClockPort>,
    pub logs: Arc<FakeLogPort>,
    pub resources: Arc<FakeResourcePort>,
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
        let exec = Executor::default();
        let service = LogService::new(
            LogRuntime {
                spawner: exec.spawner(),
                clock: clock.clone(),
            },
            config,
        );
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

    /// Queues `timeline` as the next stream.
    pub fn script(&self, timeline: Timeline<LogLine>) {
        self.logs.script().stream_logs.push_ok(timeline);
    }

    /// Opens `web-0` following its pod's life (reads the pod), settled.
    pub fn follow(&mut self, options: LogOptions) -> LogSession {
        let session = self.service.open_following_in(
            &cluster(),
            self.ports(),
            LogTarget::pod("default", "web-0"),
            options,
        );
        self.settle();
        session
    }

    /// Opens `web-0` over the log port only (no pod reads), settled.
    pub fn open_plain(&mut self, options: LogOptions) -> LogSession {
        let session = self.service.open(
            self.logs.clone(),
            LogTarget::pod("default", "web-0"),
            options,
        );
        self.settle();
        session
    }

    pub fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }

    /// Lets `total` of fake time pass in 50 ms steps, settling after each.
    pub fn run_for(&mut self, total: Duration) {
        let step = Duration::from_millis(50);
        let mut left = total;
        while !left.is_zero() {
            let by = left.min(step);
            self.clock.advance(by);
            self.settle();
            left -= by;
        }
    }

    /// The options of each `stream_logs` call, in order.
    pub fn opens(&self) -> Vec<LogOptions> {
        self.logs
            .recorded_calls()
            .into_iter()
            .map(|LogCall::StreamLogs { options, .. }| options)
            .collect()
    }
}

/// The texts of the buffer, oldest first.
pub(super) fn texts(session: &LogSession) -> Vec<String> {
    session.read(|buffer, _| buffer.iter().map(|e| e.text.to_string()).collect())
}
