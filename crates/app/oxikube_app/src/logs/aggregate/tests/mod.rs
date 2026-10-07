//! Aggregate tests over `FakeLogPort` and `FakeResourcePort` scripts, on the same deterministic
//! executor and fake clock as the single-session tests (no runtime, no threads).

mod lifecycle;
mod ordering;
mod pods;
mod sources;

use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::log::LogLine;
use oxikube_ports::LogOptions;
use oxikube_testkit::{FakeClockPort, FakeLogPort, FakeResourcePort, Timeline, deployment, pod};

use crate::logs::tests::Executor;
use crate::logs::{
    AggregatePorts, AggregateSession, AggregateSpec, LogConfig, LogRuntime, LogService,
};

/// Step the clock moves in while a test lets time pass: finer than the reorder window, so lines
/// arrive at distinct local times.
pub(super) const STEP: Duration = Duration::from_millis(20);

/// The server timestamp `ms` milliseconds into the fixture's hour.
pub(super) fn ts(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(1_760_000_000_000 + ms).unwrap()
}

/// A line of `pod`'s container `app`, stamped `ms`.
pub(super) fn line(pod: &str, ms: i64, text: &str) -> LogLine {
    LogLine::new(ts(ms), pod, "app", text)
}

/// A running pod `name` of the `web` deployment (label `app=web`, container `app`).
pub(super) fn web_pod(name: &str) -> Resource {
    pod()
        .name(name)
        .namespace("default")
        .uid(format!("uid-{name}"))
        .label("app", "web")
        .build()
}

/// The `web` deployment (selector `app=web`).
pub(super) fn web() -> Resource {
    deployment().name("web").namespace("default").build()
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

    /// Seeds the cluster with `objects`.
    pub fn seed(&self, objects: impl IntoIterator<Item = Resource>) {
        for object in objects {
            self.resources.insert(object);
        }
    }

    /// Queues `timeline` as the next container stream.
    pub fn script(&self, timeline: Timeline<LogLine>) {
        self.logs.script().stream_logs.push_ok(timeline);
    }

    /// Opens the `web` deployment of `default`, following, settled.
    pub fn open_web(&mut self) -> AggregateSession {
        let spec = AggregateSpec::of(&web_ref()).expect("a deployment");
        self.open(spec, LogOptions::follow())
    }

    pub fn open(&mut self, spec: AggregateSpec, options: LogOptions) -> AggregateSession {
        self.open_with(self.ports(), spec, options)
    }

    pub fn open_with(
        &mut self,
        ports: AggregatePorts,
        spec: AggregateSpec,
        options: LogOptions,
    ) -> AggregateSession {
        let session = self.service.open_aggregate(ports, spec, options);
        self.settle();
        session
    }

    /// Runs every task until idle.
    pub fn settle(&mut self) {
        self.exec.run(&mut self.pool);
    }

    /// Lets `total` of fake time pass in [`STEP`]s, settling after each.
    pub fn run_for(&mut self, total: Duration) {
        let mut left = total;
        while !left.is_zero() {
            let step = left.min(STEP);
            self.clock.advance(step);
            self.settle();
            left -= step;
        }
    }
}

/// The reference of the `web` deployment in the `default` namespace.
pub(super) fn web_ref() -> oxikube_domain::ids::ResourceRef {
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
    ResourceRef::namespaced(
        ClusterId::new("~/.kube/config", &ContextName::new("kind")),
        Gvk::new("apps", "v1", "Deployment"),
        "default",
        "web",
    )
}

/// The texts of the merged buffer, oldest first.
pub(super) fn texts(session: &AggregateSession) -> Vec<String> {
    session.read(|buffer, _| buffer.iter().map(|e| e.text.to_string()).collect())
}
