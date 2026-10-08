//! [`WorldLogs`]: the `LogPort` of a synthetic cluster, and [`line`], the mixed log both the
//! windowed `logs` scenario and the headless `logs-stream` scenario stream.
//!
//! Every pod's log is a [`TAIL`]-line tail, then [`LINES_PER_S`] lines a second in real time,
//! written in [`BATCH_EVERY`] slices the way a busy container's output reaches the kubelet.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream;
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::OxiResult;
use oxikube_domain::log::LogLine;
use oxikube_ports::{LogOptions, LogPort, LogStream};

/// The budget's rate: lines a pod writes per second.
pub const LINES_PER_S: u64 = 5_000;
/// Lines of the tail read before the stream (the view's default tail range reads 1 000).
pub const TAIL: usize = 1_000;
/// Every this many lines, one long line (a stack trace or a JSON payload) that wraps. 39 is odd
/// and not a multiple of 4, so it is one of the JSON lines.
pub const LONG_EVERY: usize = 40;
/// How often the stream hands over the lines written since the last slice.
const BATCH_EVERY: Duration = Duration::from_millis(4);

/// Line `k` of the log written by `pod`: a mixed stream, as a cluster's pods write one. Every 4th
/// line is plain text (a request line, a warning now and then); the rest are JSON objects in zap's
/// shape (info, with a warning, a debug or an error now and then, and every [`LONG_EVERY`]th about
/// 600 bytes of message that wraps). Timestamps are 200 µs apart from a fixed instant.
pub fn line(k: usize, pod: &str) -> LogLine {
    let at = Timestamp::from_second(1_791_115_200).unwrap_or(Timestamp::UNIX_EPOCH)
        + SignedDuration::from_micros(i64::try_from(k).unwrap_or(i64::MAX) * 200);
    LogLine::new(at, pod, "app", text(k, at))
}

fn text(k: usize, at: Timestamp) -> String {
    if k.is_multiple_of(4) {
        return match k {
            k if k % 7 == 3 => format!("WARN  GET /api/orders/{k} 200 {}ms: slow query", k % 900),
            k => format!(
                "INFO  GET /api/orders/{k} 200 {}ms user={} region=eu-west-{} trace={k:016x}",
                k % 97,
                k % 4_409,
                k % 3
            ),
        };
    }
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
}

/// The log port of a synthetic cluster: every pod writes [`line`]s at [`LINES_PER_S`], on
/// `runtime`'s timers. See the [module docs](self).
pub struct WorldLogs {
    runtime: Option<tokio::runtime::Handle>,
}

impl WorldLogs {
    /// The port, streaming on `runtime`; without one a log is its tail only.
    pub fn new(runtime: Option<tokio::runtime::Handle>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl LogPort for WorldLogs {
    async fn stream_logs(
        &self,
        _namespace: &str,
        pod: &str,
        options: &LogOptions,
    ) -> OxiResult<LogStream> {
        let tail = options
            .tail_lines
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(TAIL)
            .min(TAIL);
        let now = Timestamp::now();
        let pod = pod.to_owned();
        let tail_lines: Vec<OxiResult<LogLine>> = (TAIL - tail..TAIL)
            .map(|k| Ok(stamped(line(k, &pod), now)))
            .collect();
        let Some(runtime) = self.runtime.as_ref().filter(|_| options.follow) else {
            return Ok(stream::iter(tail_lines).boxed());
        };
        let (tx, rx) = mpsc::unbounded();
        runtime.spawn(write(tx, pod));
        Ok(stream::iter(tail_lines).chain(rx).boxed())
    }
}

/// Writes the pod's lines at [`LINES_PER_S`] until the reader goes.
async fn write(tx: mpsc::UnboundedSender<OxiResult<LogLine>>, pod: String) {
    let started = Instant::now();
    let mut written = 0u64;
    let mut tick = tokio::time::interval(BATCH_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = due_lines(started.elapsed());
        let now = Timestamp::now();
        while written < due {
            let k = TAIL + usize::try_from(written).unwrap_or(usize::MAX);
            if tx.unbounded_send(Ok(stamped(line(k, &pod), now))).is_err() {
                return;
            }
            written += 1;
        }
    }
}

/// Lines due `elapsed` into the stream at [`LINES_PER_S`].
fn due_lines(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros() * u128::from(LINES_PER_S) / 1_000_000).unwrap_or(u64::MAX)
}

/// `line` with its timestamp set to `at` (when it was written in this run).
fn stamped(line: LogLine, at: Timestamp) -> LogLine {
    LogLine::new(at, line.pod, line.container, line.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mix_has_plain_json_and_long_lines() {
        assert!(line(LONG_EVERY - 1, "p").text.len() > 500, "a long line");
        assert!(line(0, "p").text.len() < 120);
        assert!(oxikube_app::logs::parse::parse_line(&line(1, "p").text).is_some());
        assert!(oxikube_app::logs::parse::parse_line(&line(4, "p").text).is_none());
        assert_eq!(&*line(3, "web-0").pod, "web-0");
    }

    #[test]
    fn the_rate_is_5_000_lines_a_second() {
        assert_eq!(due_lines(Duration::from_secs(1)), 5_000);
        assert_eq!(due_lines(Duration::from_millis(4)), 20);
    }

    #[test]
    fn a_followed_log_streams_the_tail_then_the_rate() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let logs = WorldLogs::new(Some(runtime.handle().clone()));
        let lines = runtime.block_on(async {
            let options = LogOptions::follow().tail_lines(10);
            let stream = logs.stream_logs("ns", "firehose", &options).await.unwrap();
            let started = Instant::now();
            let lines: Vec<_> = stream
                .take_until(tokio::time::sleep(Duration::from_millis(200)))
                .collect()
                .await;
            (lines.len(), started.elapsed())
        });
        let (count, elapsed) = lines;
        let expected = 10 + due_lines(elapsed) as usize;
        assert!(count >= 10 + 500, "{count} lines in {elapsed:?}");
        assert!(count <= expected + 20, "{count} > {expected}");
    }
}
