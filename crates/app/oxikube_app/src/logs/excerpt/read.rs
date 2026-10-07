//! [`LogService::read_excerpt`]: a bounded, non-following read, driven to the end.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::{Either, select};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, LogOptions, LogSince};

use super::render::{pick, render};
use super::request::{ExcerptRequest, ExcerptSource, READ_DEADLINE};
use super::result::LogExcerpt;
use crate::logs::export::ExportFormat;
use crate::logs::{AggregatePorts, LogReader, LogService, LogState, LogTarget, SourceState};

/// Failed streams named in a [`LogExcerpt`]; the rest are counted (the notes stay short).
const MAX_FAILURES_NOTED: usize = 8;

/// Excerpts are written the way a person cites logs: server time, pod and container, text.
const FORMAT: ExportFormat = ExportFormat {
    timestamps: true,
    pod_prefix: true,
};

impl LogService {
    /// Reads the newest matching lines of `request` once and returns them as text: opens a
    /// session that does not follow, waits for it to end (at most [`READ_DEADLINE`]), picks the
    /// lines, writes them within the byte budget with secrets masked, and drops the session, which
    /// closes every stream. This is the read behind the agent's `get_logs` tool and `@logs`
    /// mention; it never streams indefinitely and reads at most `logs.max_streams` containers.
    ///
    /// Await it off the UI thread (`oxikube_runtime::spawn_kube`): the session's own task runs on
    /// the service's runtime, this future only waits for it.
    ///
    /// # Errors
    ///
    /// A validation error for a pattern that does not compile; the stream's own error (`NotFound`
    /// for a pod that does not exist, `Forbidden` for a denied `pods/log`, ...) when it failed
    /// before any line arrived.
    pub async fn read_excerpt(
        &self,
        ports: AggregatePorts,
        request: &ExcerptRequest,
    ) -> OxiResult<LogExcerpt> {
        let matcher = request
            .filter
            .compile()
            .map_err(|error| OxiError::validation(format!("grep: {error}")))?;
        let scan = request.scan_lines();
        let mut options = LogOptions {
            timestamps: true,
            tail_lines: i64::try_from(scan).ok(),
            ..LogOptions::default()
        };
        if let Some(since) = request.since {
            options.since = i64::try_from(since.as_secs()).ok().map(LogSince::Seconds);
        }

        // The session is kept until the lines are copied out: dropping it cancels the streams.
        let (reader, aggregate, _session) = match &request.source {
            ExcerptSource::Pod {
                namespace,
                pod,
                container,
            } => {
                let mut target = LogTarget::pod(namespace, pod);
                target.container.clone_from(container);
                let session = self.open(ports.logs.clone(), target, options);
                (session.reader(), None, Box::new(session) as Held)
            }
            ExcerptSource::Workload(spec) => {
                let session = self.open_aggregate(ports, spec.clone(), options);
                let view = session.aggregate().clone();
                (session.reader(), Some(view), Box::new(session) as Held)
            }
        };

        let timed_out = settle(&reader, self.clock()).await;
        let picked = reader.read(|buffer, _| pick(buffer, &matcher, request.tail));
        let state = reader.state();
        if let LogState::Failed(failure) = &state
            && picked.entries.is_empty()
        {
            return Err(OxiError::new(failure.kind, failure.message.clone())
                .with_retryable(failure.retryable));
        }

        let rendered = render(&picked.entries, FORMAT, request.max_bytes);
        let span = match (picked.entries.first(), picked.entries.last()) {
            (Some(first), Some(last)) => Some((first.ts, last.ts)),
            _ => None,
        };
        let mut excerpt = LogExcerpt {
            lines: rendered.lines,
            text: rendered.text,
            matched: picked.matched,
            scanned: picked.scanned,
            streams: 1,
            omitted: picked.matched.saturating_sub(rendered.lines),
            budget_cut: rendered.budget_cut,
            scan_limit: (!request.filter.is_empty()).then_some(scan),
            buffer_dropped: picked.buffer_dropped,
            timed_out,
            span,
            ..LogExcerpt::default()
        };
        if let LogState::Failed(failure) = state {
            excerpt
                .failures
                .push(format!("{}: {}", reader.target(), failure.message));
        }
        if let Some(view) = aggregate {
            let sources = view.sources();
            excerpt.streams = sources.len();
            excerpt.skipped_pods = view.skipped_pods();
            excerpt.matched_pods = Some(view.matched_pods());
            let failed: Vec<_> = sources
                .into_iter()
                .filter_map(|source| match source.state {
                    SourceState::Failed(failure) => Some(format!(
                        "{}/{}: {}",
                        source.pod, source.container, failure.message
                    )),
                    _ => None,
                })
                .collect();
            let more = failed.len().saturating_sub(MAX_FAILURES_NOTED);
            excerpt
                .failures
                .extend(failed.into_iter().take(MAX_FAILURES_NOTED));
            if more > 0 {
                excerpt.failures.push(format!("{more} more containers"));
            }
        }
        Ok(excerpt)
    }
}

/// The open session, held until the lines are copied out (dropping it cancels the streams).
type Held = Box<dyn std::any::Any + Send>;

/// Waits until the session reaches a terminal state or [`READ_DEADLINE`] passes on `clock`.
/// Returns whether the deadline passed first.
async fn settle(reader: &LogReader, clock: Arc<dyn ClockPort>) -> bool {
    let mut deltas = reader.deltas();
    let finished = async {
        while let Some(delta) = deltas.next().await {
            if delta.state.is_terminal() {
                break;
            }
        }
    };
    let deadline = clock.sleep(READ_DEADLINE);
    futures::pin_mut!(finished, deadline);
    matches!(select(finished, deadline).await, Either::Right(_))
}
