//! [`SearchState`]: what the search bar holds, as plain data with the rules for editing it.
//! No gpui: the view applies what it says.

use std::sync::Arc;

use oxikube_app::logs::{FilterError, LogFilter, LogMatcher};

/// What a search does to the lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// Every line stays; the matches are highlighted and next / previous jump between them.
    #[default]
    Highlight,
    /// Only the matching lines are shown (k9s's `/` filter).
    Filter,
}

impl SearchMode {
    /// The other mode.
    pub fn toggled(self) -> Self {
        match self {
            Self::Highlight => Self::Filter,
            Self::Filter => Self::Highlight,
        }
    }
}

/// What the search keeps between a view being closed and the same target being opened again in
/// the same session: the text and the toggles, never the lines. Not persisted to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    /// The text in the bar.
    pub text: String,
    /// Case-sensitive.
    pub case_sensitive: bool,
    /// Inverse.
    pub inverse: bool,
    /// Highlight or filter.
    pub mode: SearchMode,
}

/// The text, the toggles and the compiled pattern of one view's search.
///
/// Every edit compiles the pattern once. A pattern that does not compile is an
/// [`error`](Self::error) to show in the bar, and the last good one stays in effect, so typing
/// `foo(` on the way to `foo(bar)` never blanks the view.
#[derive(Debug, Default)]
pub struct SearchState {
    open: bool,
    text: String,
    case_sensitive: bool,
    inverse: bool,
    mode: SearchMode,
    /// The last pattern that compiled with its toggles (`None` when the text is empty).
    applied: Option<Arc<LogMatcher>>,
    error: Option<FilterError>,
    /// The match the user is on (a seq).
    current: Option<u64>,
}

/// What an edit asks the view to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// Nothing changed (the same text, or one that does not compile: see the error).
    Unchanged,
    /// The matcher changed: rebuild the index and the highlights.
    Rematch,
    /// Only the mode changed: the rows are rebuilt from the same index.
    Remode,
}

impl SearchState {
    /// A closed, empty search.
    pub fn new() -> Self {
        Self::default()
    }

    /// The state `saved` describes, open.
    pub fn restored(saved: &SavedSearch) -> Self {
        let mut state = Self {
            open: true,
            case_sensitive: saved.case_sensitive,
            inverse: saved.inverse,
            mode: saved.mode,
            ..Self::default()
        };
        state.set_text(&saved.text);
        state
    }

    /// What to keep for the next time the target is opened.
    pub fn saved(&self) -> SavedSearch {
        SavedSearch {
            text: self.text.clone(),
            case_sensitive: self.case_sensitive,
            inverse: self.inverse,
            mode: self.mode,
        }
    }

    /// Whether the bar is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes the bar. Closing clears the text, the matcher and the current match.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            *self = Self {
                mode: self.mode,
                case_sensitive: self.case_sensitive,
                inverse: self.inverse,
                ..Self::default()
            };
        }
    }

    /// The text in the bar.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether case matters.
    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    /// Whether the lines without the pattern are the matches.
    pub fn inverse(&self) -> bool {
        self.inverse
    }

    /// Highlight or filter.
    pub fn mode(&self) -> SearchMode {
        self.mode
    }

    /// Whether only the matching lines are shown.
    pub fn is_filtering(&self) -> bool {
        self.mode == SearchMode::Filter
    }

    /// The pattern in effect: the last one that compiled, `None` for an empty text.
    pub fn matcher(&self) -> Option<&Arc<LogMatcher>> {
        self.applied.as_ref()
    }

    /// Why the text in the bar does not compile, if it does not.
    pub fn error(&self) -> Option<&FilterError> {
        self.error.as_ref()
    }

    /// The match the user is on.
    pub fn current(&self) -> Option<u64> {
        self.current
    }

    /// Sets the match the user is on.
    pub fn set_current(&mut self, current: Option<u64>) {
        self.current = current;
    }

    /// Types `text` into the bar.
    pub fn set_text(&mut self, text: &str) -> Edit {
        if self.text == text && self.error.is_none() {
            return Edit::Unchanged;
        }
        self.text = text.to_owned();
        self.recompile()
    }

    /// Flips case sensitivity.
    pub fn toggle_case(&mut self) -> Edit {
        self.case_sensitive = !self.case_sensitive;
        self.recompile()
    }

    /// Flips inverse.
    pub fn toggle_inverse(&mut self) -> Edit {
        self.inverse = !self.inverse;
        self.recompile()
    }

    /// Flips between highlight and filter.
    pub fn toggle_mode(&mut self) -> Edit {
        self.mode = self.mode.toggled();
        Edit::Remode
    }

    fn recompile(&mut self) -> Edit {
        let filter = LogFilter {
            pattern: self.text.clone(),
            case_sensitive: self.case_sensitive,
            inverse: self.inverse,
        };
        match filter.compile() {
            Ok(matcher) => {
                self.error = None;
                self.current = None;
                self.applied = matcher.is_active().then(|| Arc::new(matcher));
                Edit::Rematch
            }
            Err(error) => {
                self.error = Some(error);
                Edit::Unchanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_compiles_once_per_edit_and_keeps_the_last_good_pattern() {
        let mut state = SearchState::new();
        assert_eq!(state.set_text("foo"), Edit::Rematch);
        assert_eq!(state.matcher().unwrap().filter().pattern, "foo");
        // `foo(` does not compile: the error shows, `foo` stays in effect.
        assert_eq!(state.set_text("foo("), Edit::Unchanged);
        assert!(state.error().unwrap().message().contains("unclosed group"));
        assert_eq!(state.matcher().unwrap().filter().pattern, "foo");
        assert_eq!(state.text(), "foo(");
        assert_eq!(state.set_text("foo(bar)"), Edit::Rematch);
        assert!(state.error().is_none());
        assert_eq!(state.matcher().unwrap().filter().pattern, "foo(bar)");
    }

    #[test]
    fn the_same_text_is_no_edit_unless_it_was_an_error() {
        let mut state = SearchState::new();
        state.set_text("a");
        assert_eq!(state.set_text("a"), Edit::Unchanged);
        state.set_text("a(");
        assert_eq!(state.set_text("a("), Edit::Unchanged);
        assert!(state.error().is_some());
    }

    #[test]
    fn an_empty_text_is_no_matcher() {
        let mut state = SearchState::new();
        state.set_text("x");
        state.set_text("");
        assert!(state.matcher().is_none());
    }

    #[test]
    fn toggles_recompile_and_reset_the_current_match() {
        let mut state = SearchState::new();
        state.set_text("err");
        state.set_current(Some(7));
        assert_eq!(state.toggle_case(), Edit::Rematch);
        assert!(state.matcher().unwrap().filter().case_sensitive);
        assert_eq!(state.current(), None);
        assert_eq!(state.toggle_inverse(), Edit::Rematch);
        assert!(state.matcher().unwrap().filter().inverse);
        assert_eq!(state.toggle_mode(), Edit::Remode);
        assert_eq!(state.mode(), SearchMode::Filter);
    }

    #[test]
    fn closing_clears_the_search_but_keeps_the_toggles() {
        let mut state = SearchState::new();
        state.set_open(true);
        state.set_text("x");
        state.toggle_case();
        state.toggle_mode();
        state.set_open(false);
        assert!(!state.is_open() && state.text().is_empty() && state.matcher().is_none());
        assert!(state.case_sensitive());
        assert_eq!(state.mode(), SearchMode::Filter);
    }

    #[test]
    fn a_saved_search_restores_open_with_its_pattern() {
        let mut state = SearchState::new();
        state.set_open(true);
        state.set_text("boom");
        state.toggle_inverse();
        let restored = SearchState::restored(&state.saved());
        assert!(restored.is_open());
        assert_eq!(restored.text(), "boom");
        assert!(restored.inverse());
        assert!(restored.matcher().is_some());
    }
}
