//! What the emulator asks of the outside world: [`GridEvent`], collected by [`GridListener`].

use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::vte::ansi::Rgb;
use bytes::Bytes;
use parking_lot::Mutex;

use super::{TermGrid, TermRgb};

/// Something the emulator needs the outside world to do, produced while parsing output.
///
/// None of these is ever logged with its payload: a reply, a title or clipboard text can carry
/// what the process printed, secrets included.
#[derive(Clone, PartialEq, Eq)]
pub enum GridEvent {
    /// Bytes to send back to the process: answers to queries such as a cursor position report
    /// (`CSI 6 n`) or device attributes (`CSI c`). The bridge writes them to the backend.
    Reply(Bytes),
    /// The process set (`Some`) or reset (`None`) the window title.
    TitleChanged(Option<Arc<str>>),
    /// The process rang the bell.
    Bell,
    /// The process asked to copy this text to the clipboard (OSC 52). Reading the clipboard is
    /// never allowed.
    ClipboardStore(String),
    /// The process asked for a palette colour the grid does not know (theme colours live in the
    /// view): answer with [`ColorRequest::reply`] and send the bytes back.
    ColorRequest(ColorRequest),
}

impl std::fmt::Debug for GridEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Payloads stay out of debug output (they may hold secrets).
        match self {
            GridEvent::Reply(bytes) => write!(f, "Reply({} bytes)", bytes.len()),
            GridEvent::TitleChanged(title) => {
                write!(
                    f,
                    "TitleChanged({})",
                    if title.is_some() { "set" } else { "reset" }
                )
            }
            GridEvent::Bell => f.write_str("Bell"),
            GridEvent::ClipboardStore(text) => write!(f, "ClipboardStore({} bytes)", text.len()),
            GridEvent::ColorRequest(request) => write!(f, "{request:?}"),
        }
    }
}

/// A process's query for a palette colour (OSC 4 / 10 / 11 / 12 with `?`).
#[derive(Clone)]
pub struct ColorRequest {
    index: usize,
    format: Arc<dyn Fn(Rgb) -> String + Send + Sync>,
}

impl ColorRequest {
    /// The colour asked for: `0..256` the indexed palette, `256` the foreground, `257` the
    /// background, `258` the cursor.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The bytes that answer the query with `colour`, in the form the process asked for.
    pub fn reply(&self, colour: TermRgb) -> Bytes {
        let rgb = Rgb {
            r: colour.r,
            g: colour.g,
            b: colour.b,
        };
        Bytes::from((self.format)(rgb).into_bytes())
    }
}

impl PartialEq for ColorRequest {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && Arc::ptr_eq(&self.format, &other.format)
    }
}

impl Eq for ColorRequest {}

impl std::fmt::Debug for ColorRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ColorRequest({})", self.index)
    }
}

/// Collects the events `Term` sends while it parses; [`TermGrid`] drains them after each call.
/// Shared between the grid and its `Term`, which only ever run under the grid's own lock, so the
/// mutex is never contended.
#[derive(Clone, Default)]
pub(super) struct GridListener(Arc<Mutex<Vec<Event>>>);

impl GridListener {
    pub(super) fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.0.lock())
    }

    /// Drops events produced by a call that is not output (scrolling, an options change).
    pub(super) fn discard(&self) {
        self.0.lock().clear();
    }
}

impl EventListener for GridListener {
    fn send_event(&self, event: Event) {
        match event {
            // Repaint hints the bridge derives from parsing itself, or that only a windowing
            // emulator acts on.
            Event::MouseCursorDirty
            | Event::Wakeup
            | Event::CursorBlinkingChange
            | Event::Exit
            | Event::ChildExit(_) => {}
            // Never honoured: a process may not read the clipboard (Osc52::OnlyCopy already
            // refuses it; this is the second lock).
            Event::ClipboardLoad(..) => {}
            event => self.0.lock().push(event),
        }
    }
}

impl TermGrid {
    /// Turns one collected `Event` into a [`GridEvent`], answering the queries the grid can
    /// answer itself.
    pub(super) fn translate(&mut self, event: Event) -> Option<GridEvent> {
        match event {
            Event::PtyWrite(text) => Some(GridEvent::Reply(Bytes::from(text.into_bytes()))),
            Event::Title(title) => {
                let title: Arc<str> = title.into();
                self.title = Some(title.clone());
                Some(GridEvent::TitleChanged(Some(title)))
            }
            Event::ResetTitle => {
                self.title = None;
                Some(GridEvent::TitleChanged(None))
            }
            Event::Bell => Some(GridEvent::Bell),
            Event::ClipboardStore(_, text) => Some(GridEvent::ClipboardStore(text)),
            Event::ColorRequest(index, format) => match self.term.colors()[index] {
                // A colour the process set itself (OSC 4): the grid knows it.
                Some(rgb) => Some(GridEvent::Reply(Bytes::from(format(rgb).into_bytes()))),
                None => Some(GridEvent::ColorRequest(ColorRequest { index, format })),
            },
            Event::TextAreaSizeRequest(format) => {
                let size = self.size;
                let window = WindowSize {
                    num_lines: size.height,
                    num_cols: size.width,
                    cell_width: size.pixel_width / size.width.max(1),
                    cell_height: size.pixel_height / size.height.max(1),
                };
                Some(GridEvent::Reply(Bytes::from(format(window).into_bytes())))
            }
            _ => None,
        }
    }
}
