//! [`TerminalState`]: one terminal session as a GPUI entity, the event-loop bridge between a
//! [`TerminalBackend`] and the grid (E09-S04).
//!
//! ```text
//! backend output ─▶ pump (tokio) ─▶ TermGrid (short lock) ─▶ wake flag ─▶ GridUpdate::Changed
//!                        │ replies                                            │ (UI thread)
//!                        ▼                                                    ▼
//! keystrokes ─▶ writer (tokio) ─▶ backend write / resize        notify_coalesced ─▶ snapshot
//! ```
//!
//! * The **pump** ([`pump::pump`]) runs on tokio through `oxikube_runtime::spawn_kube`. It parses
//!   every chunk as it arrives (a flood of megabytes is applied in full), holding the grid lock for
//!   at most [`pump::PARSE_SLICE`] bytes at a time and never across an `.await`.
//! * **Frame coalescing**: the pump sends [`GridUpdate::Changed`](pump::GridUpdate) only when it
//!   turns the shared wake flag on; the UI turns it off and calls
//!   `oxikube_runtime::notify_coalesced`, so observers (the element) hear at most one `notify` per
//!   frame however many chunks arrived. Never one per chunk.
//! * The **writer** ([`pump::write_loop`]) sends input and the emulator's replies in order, and
//!   forwards resizes through a watch channel that keeps only the latest size.
//! * **Ownership**: the entity owns both tokio tasks (abort on drop) and the drain task; dropping
//!   the entity stops the pump, closes the stream and releases the backend. Nothing is persisted:
//!   the scrollback lives in the grid, in memory (non-negotiable 5).
//!
//! The operations the element and keymap use (snapshot, resize, scroll, selection, search, input)
//! are in `ops`.

mod ops;
pub(crate) mod pump;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use futures::channel::mpsc;
use gpui::{Context, EventEmitter, Subscription, Task};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExitStatus, TerminalBackend, TerminalSize};
use oxikube_runtime::{KubeTask, NotifyCoalescedExt as _, batch_channel, spawn_kube};
use oxikube_settings::Settings as _;
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::grid::{ColorRequest, GridEvent, TermGrid};
use crate::settings::TerminalSettings;
use pump::GridUpdate;

/// Capacity of the pump → UI channel. Small on purpose: `Changed` is at most one in flight, so
/// only titles, bells and the like queue here, and a process spamming them is slowed down.
const UPDATE_CAPACITY: usize = 64;

/// What a [`TerminalState`] tells its view. Payloads are never logged (they may hold secrets).
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// The process set (`Some`) or reset (`None`) the title.
    TitleChanged(Option<Arc<str>>),
    /// The process rang the bell.
    Bell,
    /// The process asked to copy text to the clipboard (OSC 52).
    ClipboardStore(String),
    /// The process asked for a theme colour: answer with [`TerminalState::reply_color`].
    ColorRequest(ColorRequest),
    /// The transport failed (the session may still end with an exit status).
    Error(Arc<OxiError>),
    /// The process ended. The grid keeps its content until the entity is dropped.
    Exited(ExitStatus),
}

/// A terminal session: the grid, the backend it shows, and the tasks that connect them. See the
/// module docs.
///
/// Create one with `cx.new(|cx| TerminalState::new(backend, size, cx))`; the view observes it
/// (repaint) and subscribes to [`TerminalEvent`]s.
pub struct TerminalState {
    grid: Arc<Mutex<TermGrid>>,
    backend: Arc<dyn TerminalBackend>,
    input: mpsc::UnboundedSender<Bytes>,
    resize: watch::Sender<TerminalSize>,
    wake: Arc<AtomicBool>,
    exit: Option<ExitStatus>,
    last_error: Option<Arc<OxiError>>,
    _pump: KubeTask<()>,
    _writer: KubeTask<()>,
    _drain: Task<()>,
    _settings: Subscription,
}

impl EventEmitter<TerminalEvent> for TerminalState {}

impl std::fmt::Debug for TerminalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalState")
            .field("grid", &*self.grid.lock())
            .field("exit", &self.exit)
            .finish_non_exhaustive()
    }
}

impl TerminalState {
    /// Starts showing `backend` in a grid of `size`, with the scrollback from the `terminal`
    /// settings (followed live). `size` is the size the process was started with: it is not
    /// sent to the backend; later [`resize`](Self::resize)s are.
    pub fn new(
        backend: Box<dyn TerminalBackend>,
        size: TerminalSize,
        cx: &mut Context<Self>,
    ) -> Self {
        let scrollback = TerminalSettings::current(cx).scrollback_lines;
        let grid = Arc::new(Mutex::new(TermGrid::new(size, scrollback)));
        let size = grid.lock().size();
        let backend: Arc<dyn TerminalBackend> = Arc::from(backend);
        let wake = Arc::new(AtomicBool::new(false));

        let (input, input_rx) = mpsc::unbounded();
        let (resize, resize_rx) = watch::channel(size);
        let (updates, updates_rx) = batch_channel::<GridUpdate>(UPDATE_CAPACITY);

        let pump = spawn_kube(
            cx,
            pump::pump(
                backend.output_stream(),
                grid.clone(),
                input.clone(),
                updates,
                wake.clone(),
            ),
        );
        let writer = spawn_kube(cx, pump::write_loop(backend.clone(), input_rx, resize_rx));
        let drain = updates_rx.drain_into(cx, |this: &mut Self, batch, cx| {
            for update in batch {
                this.apply(update, cx);
            }
        });
        let settings = TerminalSettings::observe_in(cx, |this: &mut Self, cx| {
            let lines = TerminalSettings::current(cx).scrollback_lines;
            this.grid.lock().set_scrollback(lines);
            cx.notify();
        });

        Self {
            grid,
            backend,
            input,
            resize,
            wake,
            exit: None,
            last_error: None,
            _pump: pump,
            _writer: writer,
            _drain: drain,
            _settings: settings,
        }
    }

    /// Handles one update from the pump, on the UI thread.
    fn apply(&mut self, update: GridUpdate, cx: &mut Context<Self>) {
        match update {
            GridUpdate::Changed => {
                // Clear first: output parsed after this point flips the flag on again and sends
                // the next `Changed`; output parsed before it is in the snapshot this notify
                // leads to.
                self.wake.store(false, Ordering::Release);
                cx.notify_coalesced();
            }
            GridUpdate::Event(event) => cx.emit(match event {
                GridEvent::TitleChanged(title) => TerminalEvent::TitleChanged(title),
                GridEvent::Bell => TerminalEvent::Bell,
                GridEvent::ClipboardStore(text) => TerminalEvent::ClipboardStore(text),
                GridEvent::ColorRequest(request) => TerminalEvent::ColorRequest(request),
                // The pump sends replies to the writer, never here.
                GridEvent::Reply(_) => return,
            }),
            GridUpdate::Error(error) => {
                let error = Arc::new(error);
                self.last_error = Some(error.clone());
                cx.emit(TerminalEvent::Error(error));
                cx.notify();
            }
            GridUpdate::Exited(status) => {
                self.exit = Some(status.clone());
                cx.emit(TerminalEvent::Exited(status));
                cx.notify();
            }
        }
    }

    /// How the process ended; `None` while it runs.
    pub fn exit_status(&self) -> Option<&ExitStatus> {
        self.exit.as_ref()
    }

    /// The last transport error, if any.
    pub fn last_error(&self) -> Option<&OxiError> {
        self.last_error.as_deref()
    }

    /// Ends the session (closes the backend). The pump then reports [`TerminalEvent::Exited`].
    /// Keep the task to wait for it, or detach it.
    pub fn kill(&self, cx: &mut Context<Self>) -> KubeTask<OxiResult<()>> {
        let backend = self.backend.clone();
        spawn_kube(cx, async move { backend.kill().await })
    }
}

#[cfg(test)]
mod tests;
