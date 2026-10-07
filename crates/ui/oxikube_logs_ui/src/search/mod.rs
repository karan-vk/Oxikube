//! Search and filter in the log view (E08-S03): `/` opens a bar under the toolbar; the text is a
//! regular expression over the stored lines.
//!
//! Two modes behind one toggle: *highlight* keeps every line, paints the matches, counts them
//! (`3 of 41`) and jumps between them (enter / shift-enter, `n` / `N`, wrapping at the ends);
//! *filter* shows only the matching lines. The case toggle and the inverse toggle (the lines
//! *without* the pattern, k9s's `!`) work in both. An invalid pattern shows its reason next to the
//! field and the last good one stays in effect.
//!
//! The matching itself is [`oxikube_app::logs::LogMatcher`] / [`MatchIndex`](oxikube_app::logs::MatchIndex)
//! (the predicate the agent's `get_logs` grep reuses); this module is the view's side of it.
//!
//! | File | Holds |
//! |---|---|
//! | `state` | [`SearchState`]: the text, toggles and compiled pattern, the editing rules; [`SearchMode`], [`SavedSearch`] |
//! | `text` | [`Counts`] and the status words |
//! | `memory` | [`SearchMemory`]: a window's searches kept for the session, restored when a target is reopened |
//! | `bar` | the bar's drawing and its input events |
//! | `ops` | the operations the `logs::*` search commands call, and the requests that dispatch them |
//! | `scan` | building an index over a full buffer: inline when small, in background chunks otherwise |
//!
//! # Incremental and off the UI thread
//!
//! The view's [`LineWindow`](crate::view::LineWindow) carries the index. A delta tests only the
//! lines it appended and drops the matches of lines the ring dropped. A pattern edit builds a new
//! index: up to [`scan::INLINE_LINES`] lines on the spot (under a frame's budget), otherwise in
//! chunks on the background executor, publishing the finished index in one update. Until then the
//! old index keeps serving, so typing never blocks a frame. Highlight spans are computed for the
//! rows on screen at draw time.
//!
//! # Per session
//!
//! A view's search is saved in the window's [`SearchMemory`] on every change (and forgotten when
//! the bar is closed): reopening the same target in the same session restores it.

mod bar;
mod memory;
mod ops;
mod scan;
mod state;
mod text;

use gpui::{Entity, Subscription, Task};
use oxikube_ui::input::InputState;

pub use memory::SearchMemory;
pub use state::{Edit, SavedSearch, SearchMode, SearchState};
pub use text::{Counts, status_text};

/// What a [`LogView`](crate::LogView) keeps for its search beyond [`SearchState`].
#[derive(Default)]
pub(crate) struct Search {
    pub(crate) state: SearchState,
    /// The bar's text field, made when the bar is first shown (it needs a window).
    pub(crate) input: Option<Entity<InputState>>,
    pub(crate) subscription: Option<Subscription>,
    /// Whether the field has the keyboard focus (the key context says `Editing`).
    pub(crate) editing: bool,
    /// Whether an index is being built in the background.
    pub(crate) scanning: bool,
    /// The background build; replaced (cancelling it) by the next, never cleared from inside.
    pub(crate) scan: Option<Task<()>>,
    /// Where the search is kept for the session.
    pub(crate) memory: Option<SearchMemory>,
}
