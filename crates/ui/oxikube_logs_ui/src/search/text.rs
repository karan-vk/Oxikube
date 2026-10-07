//! The words of the search bar's status.

use super::state::{SearchMode, SearchState};
use crate::view::text::group;

/// The numbers the status is worded from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Matching lines retained.
    pub matches: usize,
    /// The 1-based position of the current match among them, when the user is on one.
    pub ordinal: Option<usize>,
    /// Lines retained.
    pub lines: usize,
    /// Whether an index is still being built (a pattern edit over a large buffer).
    pub scanning: bool,
}

/// The words next to the field for `state` and `counts`: the error, or `Searching…`, or the
/// count (`3 of 41`, `41 matches`, `No matches`), or how many lines the filter shows.
pub fn status_text(state: &SearchState, counts: Counts) -> String {
    if let Some(error) = state.error() {
        return format!("Invalid pattern: {}", error.message());
    }
    if state.matcher().is_none() {
        return String::new();
    }
    if counts.scanning {
        return "Searching…".to_owned();
    }
    let Counts {
        matches,
        ordinal,
        lines,
        ..
    } = counts;
    match state.mode() {
        SearchMode::Filter => format!("{} of {} lines", group(matches as u64), group(lines as u64)),
        SearchMode::Highlight => match (matches, ordinal) {
            (0, _) => "No matches".to_owned(),
            (n, Some(ordinal)) => format!("{} of {}", group(ordinal as u64), group(n as u64)),
            (1, None) => "1 match".to_owned(),
            (n, None) => format!("{} matches", group(n as u64)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(matches: usize, ordinal: Option<usize>) -> Counts {
        Counts {
            matches,
            ordinal,
            lines: 1_000,
            scanning: false,
        }
    }

    #[test]
    fn the_status_words() {
        let mut state = SearchState::new();
        assert_eq!(
            status_text(&state, counts(0, None)),
            "",
            "no pattern, no words"
        );
        state.set_text("err");
        assert_eq!(status_text(&state, counts(0, None)), "No matches");
        assert_eq!(status_text(&state, counts(1, None)), "1 match");
        assert_eq!(status_text(&state, counts(1_041, None)), "1,041 matches");
        assert_eq!(status_text(&state, counts(41, Some(3))), "3 of 41");
        let scanning = Counts {
            scanning: true,
            ..counts(5, None)
        };
        assert_eq!(status_text(&state, scanning), "Searching…");
        state.toggle_mode();
        assert_eq!(
            status_text(&state, counts(41, Some(3))),
            "41 of 1,000 lines"
        );
        state.set_text("err(");
        assert_eq!(
            status_text(&state, counts(41, None)),
            "Invalid pattern: unclosed group"
        );
    }
}
