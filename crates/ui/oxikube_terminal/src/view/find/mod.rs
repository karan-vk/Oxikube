//! Find in scrollback (E09-S11): `terminal::Search` opens a bar above the terminal; the text is a
//! regular expression (smart case) over the screen and the scrollback.
//!
//! Matches are painted over the cells (`search_match`, the current one `search_active_match`);
//! enter / shift-enter, `terminal::SearchNext` / `SearchPrevious` and the arrows step through
//! them, wrapping at the ends and scrolling the history to the match. An invalid pattern shows
//! its reason in the bar and the last good matches stay.
//!
//! The scan is [`TerminalState::search`]: sliced on the background executor, output held back
//! only while it runs. While the bar is open and the process keeps printing, the matches are
//! looked up again at most every [`REFRESH_DELAY`] so they follow the lines as they scroll away.
//! The pattern is never logged (the user may be searching for a secret) and never saved.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`Find`]: the matches and the current one; the operations the `terminal::Search*` actions call |
//! | `bar` | the bar's drawing and its input events |
//! | `carry` | which match of a new scan is the one the user was on |

mod bar;
mod carry;

use std::rc::Rc;
use std::time::Duration;

use gpui::{Context, Entity, SharedString, Subscription, Task, Window};
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ui::input::InputState;

use super::TerminalView;
use crate::element::SearchHighlights;
use crate::grid::GridMatch;
use crate::state::SearchResult;

/// The longest the matches may lag behind output while the bar is open.
pub const REFRESH_DELAY: Duration = Duration::from_millis(300);

/// What a terminal view keeps for its search.
#[derive(Default)]
pub(super) struct Find {
    open: bool,
    /// The pattern the current matches are for.
    pattern: String,
    /// Every match in grid order.
    matches: Rc<Vec<GridMatch>>,
    current: Option<usize>,
    /// Why the pattern could not be searched; the last good matches stay.
    error: Option<SharedString>,
    /// The bar's field, made when the bar is first shown (a field needs a window).
    input: Option<Entity<InputState>>,
    subscription: Option<Subscription>,
    /// The scan in flight; replaced (cancelled) by the next one.
    scan: Option<Task<()>>,
    /// The wait before the matches are looked up again after output.
    refresh: Option<Task<()>>,
    refresh_pending: bool,
    /// The content generation the matches were scanned at.
    seen_generation: u64,
    /// Lines of history the grid had when the matches were scanned.
    scanned_history: usize,
}

impl Find {
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// What the element paints, when there are matches.
    pub(super) fn highlights(&self) -> Option<SearchHighlights> {
        if !self.open || self.matches.is_empty() {
            return None;
        }
        Some(SearchHighlights {
            matches: self.matches.clone(),
            current: self.current,
        })
    }

    fn forget_matches(&mut self) {
        self.matches = Rc::default();
        self.current = None;
    }
}

/// How a finished scan treats the current match.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// A new pattern: land on the match nearest the bottom and scroll to it.
    Typed,
    /// The same pattern again after output: keep the position, never scroll.
    Refresh,
}

impl TerminalView {
    /// The pattern being searched, for tests and the status line.
    pub fn search_pattern(&self) -> &str {
        &self.find.pattern
    }

    /// Whether the search bar is open.
    pub fn search_open(&self) -> bool {
        self.find.is_open()
    }

    /// The matches of the current search, in grid order.
    pub fn search_matches(&self) -> &[GridMatch] {
        &self.find.matches
    }

    /// Why the pattern cannot be searched (never the pattern itself), while it cannot.
    pub fn search_error(&self) -> Option<&str> {
        self.find.error.as_deref()
    }

    /// The match the user is on, as an index into [`search_matches`](Self::search_matches).
    pub fn search_current(&self) -> Option<usize> {
        self.find.current
    }

    /// `terminal::Search`: shows the bar and puts the keyboard in its field, with the last
    /// pattern selected and searched again (its matches were dropped when the bar closed).
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal().is_none() {
            return;
        }
        let input = self.ensure_search_input(window, cx);
        self.find.open = true;
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        if !self.find.pattern.is_empty() {
            // The field keeps its text, so no edit event comes to search it again.
            self.scan(Scan::Typed, cx);
        }
        cx.notify();
    }

    /// `terminal::SearchClose`: hides the bar, drops the matches and gives the keyboard back to
    /// the process.
    pub fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find.open {
            return;
        }
        self.find.open = false;
        self.find.forget_matches();
        self.find.error = None;
        self.find.scan = None;
        self.find.refresh = None;
        self.find.refresh_pending = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// `terminal::SearchNext`: the match after the current one, wrapping.
    pub fn search_next(&mut self, cx: &mut Context<Self>) {
        self.step_search(1, cx);
    }

    /// `terminal::SearchPrevious`: the match before the current one, wrapping.
    pub fn search_previous(&mut self, cx: &mut Context<Self>) {
        self.step_search(-1, cx);
    }

    fn step_search(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.find.matches.len();
        if count == 0 {
            return;
        }
        let next = match self.find.current {
            Some(current) => (current as isize + by).rem_euclid(count as isize) as usize,
            None if by > 0 => 0,
            None => count - 1,
        };
        self.find.current = Some(next);
        self.reveal_current(cx);
        cx.notify();
    }

    /// Scrolls the history so the current match is on screen.
    fn reveal_current(&mut self, cx: &mut Context<Self>) {
        let (Some(found), Some(state)) = (
            self.find
                .current
                .and_then(|index| self.find.matches.get(index)),
            self.terminal().cloned(),
        ) else {
            return;
        };
        let start = found.start;
        state.update(cx, |state, cx| state.scroll_to(start, cx));
    }

    /// The pattern in the field changed: scan again.
    pub(super) fn search_edited(&mut self, text: &str, cx: &mut Context<Self>) {
        if text == self.find.pattern && self.find.error.is_none() {
            return;
        }
        self.find.pattern = text.to_owned();
        self.find.error = None;
        if text.is_empty() {
            self.find.forget_matches();
            self.find.scan = None;
            cx.notify();
            return;
        }
        self.scan(Scan::Typed, cx);
    }

    /// Starts a scan of the screen and scrollback for the current pattern; the result replaces
    /// the matches when it arrives.
    fn scan(&mut self, kind: Scan, cx: &mut Context<Self>) {
        let Some(state) = self.terminal().cloned() else {
            return;
        };
        self.find.seen_generation = state.read(cx).content_generation();
        let pattern = self.find.pattern.clone();
        let scanned = state.update(cx, |state, cx| state.search_anchored(&pattern, cx));
        self.find.scan = Some(cx.spawn(async move |this, cx| {
            let result = scanned.await;
            this.update(cx, |this, cx| {
                if this.find.pattern == pattern {
                    this.scanned(kind, result, cx);
                }
            })
            .ok();
        }));
    }

    fn scanned(&mut self, kind: Scan, result: OxiResult<SearchResult>, cx: &mut Context<Self>) {
        match result {
            Ok(SearchResult {
                matches,
                history_size,
            }) => {
                // The scan returns the matches top to bottom already.
                let previous = self
                    .find
                    .current
                    .and_then(|index| self.find.matches.get(index).copied());
                self.find.error = None;
                self.find.current = match kind {
                    Scan::Typed => matches.len().checked_sub(1),
                    // Stay on the match the user is on, at the line output moved it to.
                    Scan::Refresh => match previous {
                        Some(old) => {
                            let shift = history_size.saturating_sub(self.find.scanned_history);
                            carry::carry_current(old, shift, &matches)
                        }
                        // Nothing was matched before: the first matches start at the bottom.
                        None => matches.len().checked_sub(1),
                    },
                };
                self.find.scanned_history = history_size;
                self.find.matches = Rc::new(matches);
                if kind == Scan::Typed {
                    self.reveal_current(cx);
                }
            }
            Err(error) if error.kind() == ErrorKind::Validation => {
                // The pattern, not the text: say what is wrong, keep the last good matches.
                self.find.error = Some("Invalid regular expression".into());
            }
            Err(_) => self.find.error = Some("The search failed".into()),
        }
        cx.notify();
    }

    /// The terminal changed: with the bar open, look the matches up again soon after new output, a clear or a resize.
    pub(super) fn find_on_terminal_change(&mut self, cx: &mut Context<Self>) {
        if !self.find.open || self.find.pattern.is_empty() || self.find.refresh_pending {
            return;
        }
        let Some(state) = self.terminal() else {
            return;
        };
        if state.read(cx).content_generation() == self.find.seen_generation {
            return;
        }
        self.find.refresh_pending = true;
        self.find.refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DELAY).await;
            this.update(cx, |this, cx| {
                this.find.refresh_pending = false;
                if this.find.open && !this.find.pattern.is_empty() {
                    this.scan(Scan::Refresh, cx);
                }
            })
            .ok();
        }));
    }
}
