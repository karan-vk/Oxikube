//! [`FakeTerminalBackend`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::{self, BoxStream};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{BackendEvent, ExitStatus, TerminalBackend, TerminalSize};
use parking_lot::Mutex;

use crate::fakes::FakeClockPort;
use crate::script::Timeline;

/// One call made on a [`FakeTerminalBackend`], in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalCall {
    /// `write(bytes)`.
    Write(Vec<u8>),
    /// `resize(size)`.
    Resize(TerminalSize),
    /// `output_stream()`.
    OutputStream,
    /// `kill()`.
    Kill,
}

struct State {
    calls: Vec<TerminalCall>,
    echo: bool,
    /// No more input or output: the process exited or was killed.
    closed: bool,
    /// Sender of live events (echo, `output`, `exit`); dropped when closed.
    live: Option<mpsc::UnboundedSender<BackendEvent>>,
    live_rx: Option<mpsc::UnboundedReceiver<BackendEvent>>,
    scripted: Option<Timeline<BackendEvent>>,
    fail_writes: Option<OxiError>,
}

struct Inner {
    state: Mutex<State>,
    clock: Mutex<Arc<FakeClockPort>>,
}

/// A scripted [`TerminalBackend`]. See the module docs.
///
/// * **Echo** (the default for [`echo`](Self::echo)): everything written comes back as
///   [`BackendEvent::Output`], like a terminal in cooked mode. [`silent`](Self::silent)
///   turns it off.
/// * **Scripted events**: [`output_at`](Self::output_at), [`error_at`](Self::error_at) and
///   [`exit_at`](Self::exit_at) queue events at offsets measured from the moment
///   `output_stream()` is called, on [`clock`](Self::clock); nothing sleeps for real.
/// * **Live events**: [`output`](Self::output), [`error`](Self::error) and
///   [`exit`](Self::exit) push an event now.
/// * **Ending**: an `Exited` event (scripted or live) or [`kill`](TerminalBackend::kill)
///   ends the output stream and makes later writes fail with `Conflict`. `kill` is
///   idempotent and emits `Exited` (signal `KILL`) when no exit was seen.
/// * **Recording**: [`calls`](Self::calls), [`writes`](Self::writes),
///   [`resizes`](Self::resizes), [`kill_count`](Self::kill_count).
///
/// Like the real backends, `output_stream` is single-consumer: later calls return an
/// empty stream.
#[derive(Clone)]
pub struct FakeTerminalBackend {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for FakeTerminalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.state.lock();
        f.debug_struct("FakeTerminalBackend")
            .field("echo", &state.echo)
            .field("closed", &state.closed)
            .field("calls", &state.calls.len())
            .finish()
    }
}

impl Default for FakeTerminalBackend {
    fn default() -> Self {
        Self::echo()
    }
}

impl FakeTerminalBackend {
    fn build(echo: bool, clock: Arc<FakeClockPort>) -> Self {
        let (tx, rx) = mpsc::unbounded();
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    calls: Vec::new(),
                    echo,
                    closed: false,
                    live: Some(tx),
                    live_rx: Some(rx),
                    scripted: None,
                    fail_writes: None,
                }),
                clock: Mutex::new(clock),
            }),
        }
    }

    /// A backend that echoes every write back as output.
    pub fn echo() -> Self {
        Self::build(true, Arc::new(FakeClockPort::default()))
    }

    /// A backend that produces only what the test scripts or emits.
    pub fn silent() -> Self {
        Self::build(false, Arc::new(FakeClockPort::default()))
    }

    /// Times scripted events on `clock` instead of a private one. Call before handing the
    /// backend out.
    #[must_use]
    pub fn with_clock(self, clock: Arc<FakeClockPort>) -> Self {
        *self.inner.clock.lock() = clock;
        self
    }

    /// The clock scripted events are timed on.
    pub fn clock(&self) -> Arc<FakeClockPort> {
        self.inner.clock.lock().clone()
    }

    fn script(self, f: impl FnOnce(Timeline<BackendEvent>) -> Timeline<BackendEvent>) -> Self {
        {
            let mut state = self.inner.state.lock();
            let timeline = state.scripted.take().unwrap_or_default();
            state.scripted = Some(f(timeline));
        }
        self
    }

    /// Scripts `bytes` as output `at` after the output stream starts.
    #[must_use]
    pub fn output_at(self, at: Duration, bytes: impl Into<Bytes>) -> Self {
        self.script(|t| t.ok_at(at, BackendEvent::Output(bytes.into())))
    }

    /// Scripts a transport error `at` after the output stream starts.
    #[must_use]
    pub fn error_at(self, at: Duration, error: OxiError) -> Self {
        self.script(|t| t.err_at(at, error))
    }

    /// Scripts the process exit `at` after the output stream starts; the stream ends there.
    #[must_use]
    pub fn exit_at(self, at: Duration, status: ExitStatus) -> Self {
        self.script(|t| t.ok_at(at, BackendEvent::Exited(status)))
    }

    /// Makes every following `write` fail with `error` (one error per call, then writes
    /// succeed again).
    pub fn fail_next_write(&self, error: OxiError) {
        self.inner.state.lock().fail_writes = Some(error);
    }

    fn push(&self, event: BackendEvent) -> bool {
        let mut state = self.inner.state.lock();
        if state.closed {
            return false;
        }
        let ends = matches!(event, BackendEvent::Exited(_));
        if let Some(tx) = &state.live {
            let _ = tx.unbounded_send(event);
        }
        if ends {
            state.closed = true;
            state.live = None;
        }
        true
    }

    /// Emits output now. Returns `false` when the session already ended.
    pub fn output(&self, bytes: impl Into<Bytes>) -> bool {
        self.push(BackendEvent::Output(bytes.into()))
    }

    /// Emits a transport error now. Returns `false` when the session already ended.
    pub fn error(&self, error: OxiError) -> bool {
        self.push(BackendEvent::Error(error))
    }

    /// Ends the session with `status` now. Returns `false` when it already ended.
    pub fn exit(&self, status: ExitStatus) -> bool {
        self.push(BackendEvent::Exited(status))
    }

    /// Every call made so far, in order.
    pub fn calls(&self) -> Vec<TerminalCall> {
        self.inner.state.lock().calls.clone()
    }

    /// The bytes of each `write`, in order.
    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                TerminalCall::Write(bytes) => Some(bytes),
                _ => None,
            })
            .collect()
    }

    /// Everything written, concatenated.
    pub fn written(&self) -> Vec<u8> {
        self.writes().concat()
    }

    /// The size of each `resize`, in order.
    pub fn resizes(&self) -> Vec<TerminalSize> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                TerminalCall::Resize(size) => Some(size),
                _ => None,
            })
            .collect()
    }

    /// How many times `kill` was called (it is idempotent, the count is not).
    pub fn kill_count(&self) -> usize {
        self.calls()
            .iter()
            .filter(|call| matches!(call, TerminalCall::Kill))
            .count()
    }

    /// Whether the session ended (exit, scripted exit seen by the stream, or kill).
    pub fn is_closed(&self) -> bool {
        self.inner.state.lock().closed
    }
}

#[async_trait]
impl TerminalBackend for FakeTerminalBackend {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        let mut state = self.inner.state.lock();
        state.calls.push(TerminalCall::Write(bytes.to_vec()));
        if let Some(error) = state.fail_writes.take() {
            return Err(error);
        }
        if state.closed {
            return Err(OxiError::conflict("the terminal session ended"));
        }
        if state.echo
            && let Some(tx) = &state.live
        {
            let _ = tx.unbounded_send(BackendEvent::Output(Bytes::copy_from_slice(bytes)));
        }
        Ok(())
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        let mut state = self.inner.state.lock();
        state.calls.push(TerminalCall::Resize(size));
        if state.closed {
            return Err(OxiError::conflict("the terminal session ended"));
        }
        Ok(())
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        let (live, scripted) = {
            let mut state = self.inner.state.lock();
            state.calls.push(TerminalCall::OutputStream);
            (state.live_rx.take(), state.scripted.take())
        };
        let Some(live) = live else {
            return stream::empty().boxed();
        };
        let timeline = scripted
            .unwrap_or_default()
            .replay(self.clock())
            .map(|item| item.unwrap_or_else(BackendEvent::Error));
        let inner = self.inner.clone();
        // The first `Exited` ends the stream and closes the session, wherever it came from.
        stream::select(live, timeline)
            .scan(false, move |ended, event| {
                if *ended {
                    return futures::future::ready(None);
                }
                if matches!(event, BackendEvent::Exited(_)) {
                    *ended = true;
                    let mut state = inner.state.lock();
                    state.closed = true;
                    state.live = None;
                }
                futures::future::ready(Some(event))
            })
            .boxed()
    }

    async fn kill(&self) -> OxiResult<()> {
        let mut state = self.inner.state.lock();
        state.calls.push(TerminalCall::Kill);
        if !state.closed {
            state.closed = true;
            if let Some(tx) = state.live.take() {
                let _ = tx.unbounded_send(BackendEvent::Exited(ExitStatus::killed_by("KILL")));
            }
        }
        Ok(())
    }
}
