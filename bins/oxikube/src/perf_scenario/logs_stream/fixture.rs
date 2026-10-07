//! The fake log stream behind `logs-stream`: a connected cluster on testkit fakes with one pod
//! whose log is a [`TAIL`]-line tail followed by [`LINES_PER_S`] lines a second, replayed on the
//! log port's clock.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::App;
use jiff::{SignedDuration, Timestamp};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::log::LogLine;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort, Timeline,
};
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

/// One frame at 120 Hz.
pub const FRAME: Duration = Duration::from_micros(8_333);
/// The budget's rate: lines the pod writes per second.
pub const LINES_PER_S: u64 = 5_000;
/// Lines of the tail read before the stream (the view's default tail range reads 1 000).
pub const TAIL: usize = 1_000;
/// Every this many lines, one long line (a stack trace or a JSON payload) that wraps. 39 is odd
/// and not a multiple of 4, so it is one of the JSON lines.
const LONG_EVERY: usize = 40;

/// The pod's name.
const POD: &str = "firehose";

/// Line `k` of the log: a mixed stream, as a cluster's pods write one. Every 4th line is plain
/// text (a request line, a warning now and then); the rest are JSON objects in zap's shape (info,
/// with a warning, a debug or an error now and then, and every [`LONG_EVERY`]th about 600 bytes
/// of message that wraps).
pub fn line(k: usize) -> LogLine {
    let at = Timestamp::from_second(1_791_115_200).unwrap_or(Timestamp::UNIX_EPOCH)
        + SignedDuration::from_micros(i64::try_from(k).unwrap_or(i64::MAX) * 200);
    let text = if k % 4 == 0 {
        match k {
            k if k % 7 == 3 => format!("WARN  GET /api/orders/{k} 200 {}ms: slow query", k % 900),
            k => format!(
                "INFO  GET /api/orders/{k} 200 {}ms user={} region=eu-west-{} trace={k:016x}",
                k % 97,
                k % 4_409,
                k % 3
            ),
        }
    } else {
        let (level, message) = match k {
            k if k % LONG_EVERY == LONG_EVERY - 1 => (
                "error",
                format!(
                    "request {k} failed: {}",
                    "upstream payments-svc refused the connection; ".repeat(12)
                ),
            ),
            k if k % 7 == 3 => ("warn", format!("GET /api/orders/{k} slow query")),
            k if k % 11 == 5 => ("debug", format!("cache miss for order {k}")),
            k => ("info", format!("GET /api/orders/{k} 200")),
        };
        format!(
            r#"{{"level":"{level}","ts":{}.{:06},"caller":"orders/handler.go:88","msg":"{message}","status":200,"latency_ms":{},"user":{},"region":"eu-west-{}","trace":"{k:016x}"}}"#,
            at.as_second(),
            at.subsec_nanosecond() / 1_000,
            k % 97,
            k % 4_409,
            k % 3
        )
    };
    LogLine::new(at, POD, "app", text)
}

/// The commands of the view go nowhere in the scenario.
pub struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

/// One connected cluster on fakes with the pod and its log stream scripted.
pub struct Fixture {
    pub sessions: ClusterSessionManager,
    pub target: ResourceRef,
    ports: FakeClusterPorts,
}

impl Fixture {
    /// The cluster, connected; the pod's log streams for `frames` frames of [`FRAME`] after its
    /// tail (and stays open).
    pub fn connected(frames: usize) -> Result<Self> {
        let context = ContextName::new("perf");
        let cluster = ClusterId::new("/perf/kubeconfig", &context);
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let ports = connector.ports_for(&cluster);
        ports.resources.insert(pod()?);
        let streamed = streamed_lines(frames);
        let tail = Timeline::immediate((0..TAIL).map(line));
        let timeline = (0..streamed).fold(tail, |timeline, k| {
            let at = Duration::from_micros(
                u64::try_from(k).unwrap_or(u64::MAX) * 1_000_000 / LINES_PER_S,
            );
            timeline.ok_at(at, line(TAIL + k))
        });
        ports
            .logs
            .script()
            .stream_logs
            .push_ok(timeline.keep_open());
        let source = Arc::new(
            FakeClusterSourcePort::new().with_contexts([ClusterContext::new(
                cluster.clone(),
                context,
                SourceId("kubeconfig".into()),
            )]),
        );
        let sessions =
            ClusterSessionManager::new(connector, source, Arc::new(FakeClockPort::default()));
        futures::executor::block_on(sessions.connect(&cluster)).context("connecting")?;
        let target = ResourceRef::namespaced(cluster, Gvk::new("", "v1", "Pod"), "perf", POD);
        Ok(Self {
            sessions,
            target,
            ports,
        })
    }

    /// The clock the log is replayed on (and the `LogService` flushes on): advancing it a
    /// [`FRAME`] delivers that frame's lines.
    pub fn log_clock(&self) -> Arc<FakeClockPort> {
        self.ports.logs.clock().clone()
    }
}

/// Lines written in `frames` frames at [`LINES_PER_S`].
pub fn streamed_lines(frames: usize) -> usize {
    let micros = FRAME.as_micros() * frames as u128;
    usize::try_from(micros * u128::from(LINES_PER_S) / 1_000_000).unwrap_or(usize::MAX)
}

/// The pod: one container.
fn pod() -> Result<Resource> {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": POD, "namespace": "perf"},
        "spec": {"containers": [{"name": "app"}]},
        "status": {"phase": "Running"}
    }))
    .context("the pod")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stream_writes_5_000_lines_a_second() {
        let per_second = streamed_lines(120);
        assert!((4_990..=5_000).contains(&per_second), "{per_second}");
        assert!(line(LONG_EVERY - 1).text.len() > 500, "a long line");
        assert!(line(0).text.len() < 120);
        assert!(
            oxikube_app::logs::parse::parse_line(&line(1).text).is_some(),
            "JSON"
        );
        assert!(oxikube_app::logs::parse::parse_line(&line(LONG_EVERY - 1).text).is_some());
        assert!(
            oxikube_app::logs::parse::parse_line(&line(4).text).is_none(),
            "plain"
        );
    }
}
