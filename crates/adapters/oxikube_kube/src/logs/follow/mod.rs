// Portions derived from kdash (https://github.com/kdash-rs/kdash), `src/network/stream.rs` at
// commit c303673 (v2.1.1): `stream_container_logs` (reconnect loop with a `since_seconds`
// overlap, backoff, dedup against recent lines, size and time batching) and
// `fetch_previous_logs`. MIT licence; the full text follows. Modifications (c) Oxikube
// contributors: the loop yields `LogLine`s through a bounded channel instead of locking app
// state; reconnects resume from the last kubelet timestamp rather than the client clock;
// dedup keys on (timestamp, text) over a replay window instead of the text of the last 50
// lines; a restarted container's previous instance is read to close the gap; the loop ends
// when the pod is gone or the container finished instead of reconnecting forever.
//
// Copyright (c) 2021 Deepu K Sasidharan
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! The reconnect loop for one container: open, read, dedup, batch, and on a dropped stream
//! check the pod and open again from a little before the last line seen.
//!
//! ```text
//! open --> read lines --> dedup --> batch --> channel
//!   ^          | EOF or error
//!   |          v
//!   |     pod gone / container finished?  --yes--> end
//!   |          | no
//!   |          v
//!   +---- backoff, restarted? read the previous instance first
//! ```
//!
//! Why no line is lost or doubled across a restart: the follow stream of a container ends
//! when the container does, after the server has sent every line up to then, so the next
//! stream only needs to start where the last one stopped. The reconnect asks for
//! `sinceTime = last timestamp - overlap` (the server rounds it to whole seconds) and
//! [`Dedup`] drops the replayed lines by `(timestamp, text)`. When the restart count rose
//! while we were disconnected, the lines the old instance wrote in the gap sit in its
//! `previous` log, which is read (with the same overlap) before following the new instance.

use std::io;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::OxiError;
use oxikube_domain::log::LogLine;
use oxikube_ports::LogOptions;
use tokio::time::{sleep, timeout_at};

mod resume;

use super::config::LogsConfig;
use super::dedup::Dedup;
use super::line::{Line, LineReader};
use super::source::{LogSource, OpenRequest, PodInfo, Reader};
use super::stream::{Closed, Sink};

/// The container a follower reads.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub(crate) namespace: String,
    pub(crate) pod: Arc<str>,
    pub(crate) container: Arc<str>,
}

/// How one response ended.
enum End {
    Eof,
    Failed(io::Error),
    Closed,
}

/// What to do after a failed open.
enum Reopen {
    Open(Reader),
    /// Try again after a pause.
    Wait,
    Stop,
}

/// Reads one container's log through reconnects.
pub(crate) struct Follower {
    source: Arc<dyn LogSource>,
    config: LogsConfig,
    target: Target,
    options: LogOptions,
    /// Whether to reopen after the stream ends (follow without a byte limit, not `previous`).
    reconnect: bool,
    sink: Sink,
    dedup: Dedup,
    /// The pod instance being followed; a pod recreated under the same name ends the stream.
    uid: Option<String>,
    /// Restart count last seen, to notice a restart while disconnected.
    restarts: Option<i32>,
    replay_previous: bool,
    emitted: u64,
}

/// The request for the first open of `options`.
pub(crate) fn first_request(container: &str, options: &LogOptions) -> OpenRequest {
    OpenRequest {
        container: container.to_owned(),
        follow: options.follow && !options.previous,
        previous: options.previous,
        since: options.since,
        tail_lines: options.tail_lines,
        limit_bytes: options.limit_bytes,
    }
}

impl Follower {
    /// `pod` is what is known of the pod now (its uid and restart count seed the checks).
    pub(crate) fn new(
        source: Arc<dyn LogSource>,
        config: LogsConfig,
        target: Target,
        options: LogOptions,
        sink: Sink,
        pod: Option<&PodInfo>,
    ) -> Self {
        let restarts = pod
            .and_then(|p| p.container(&target.container))
            .map(|c| c.restart_count);
        Self {
            dedup: Dedup::new(config.dedup_window),
            reconnect: options.follow && !options.previous && options.limit_bytes.is_none(),
            uid: pod.map(|p| p.uid.clone()),
            restarts,
            replay_previous: false,
            emitted: 0,
            source,
            config,
            target,
            options,
            sink,
        }
    }

    /// Reads until the log ends for good, the pod goes away, an error is final, or the
    /// consumer is gone. `first` is an already open response, if any.
    pub(crate) async fn run(mut self, first: Option<Reader>) {
        let mut next = first;
        let mut failures = 0u32;
        let mut idle = 0u32;
        loop {
            let reader = match next.take() {
                Some(reader) => reader,
                None => match self.reopen(&mut failures).await {
                    Reopen::Open(reader) => reader,
                    Reopen::Wait => {
                        sleep(self.config.backoff(idle)).await;
                        idle = idle.saturating_add(1);
                        continue;
                    }
                    Reopen::Stop => return,
                },
            };
            let before = self.emitted;
            let end = self.pump(reader).await;
            let clean = match end {
                End::Closed => return,
                End::Eof if !self.reconnect => return,
                End::Failed(err) if !self.reconnect => {
                    let _ = self.sink.fail(read_error(&err)).await;
                    return;
                }
                End::Eof => true,
                End::Failed(err) => {
                    tracing::debug!(kind = ?err.kind(), "log stream broke; reconnecting");
                    false
                }
            };
            idle = if self.emitted > before {
                0
            } else {
                idle.saturating_add(1)
            };
            if !self.should_reconnect(clean).await {
                return;
            }
            sleep(self.config.backoff(idle.saturating_sub(1))).await;
        }
    }

    /// Reads lines until the response ends, flushing a partial batch on the timer.
    async fn pump(&mut self, reader: Reader) -> End {
        let mut lines = LineReader::new(reader);
        loop {
            let next = match self.sink.deadline() {
                Some(deadline) => match timeout_at(deadline, lines.next_line()).await {
                    Ok(next) => next,
                    Err(_) => {
                        if self.sink.flush().await.is_err() {
                            return End::Closed;
                        }
                        continue;
                    }
                },
                None => lines.next_line().await,
            };
            match next {
                Ok(Some(line)) => {
                    if self.deliver(line).await.is_err() {
                        return End::Closed;
                    }
                }
                Ok(None) => {
                    return match self.sink.flush().await {
                        Ok(()) => End::Eof,
                        Err(Closed) => End::Closed,
                    };
                }
                Err(err) => {
                    return match self.sink.flush().await {
                        Ok(()) => End::Failed(err),
                        Err(Closed) => End::Closed,
                    };
                }
            }
        }
    }

    async fn deliver(&mut self, line: Line) -> Result<(), Closed> {
        let ts = line
            .ts
            .or_else(|| self.dedup.newest())
            .unwrap_or_else(Timestamp::now);
        if !self.dedup.admit(ts, line.key) {
            return Ok(());
        }
        self.emitted += 1;
        let mut out = LogLine::new(
            ts,
            Arc::clone(&self.target.pod),
            Arc::clone(&self.target.container),
            line.text,
        );
        out.truncated |= line.cut;
        self.sink.push(out).await
    }
}

/// A read error mid-stream, without anything the server sent.
fn read_error(err: &io::Error) -> OxiError {
    OxiError::network(format!("the log stream broke: {:?}", err.kind()))
}
