//! Fills that text is drawn over, chosen so the text on them keeps WCAG AA contrast (checked
//! in `contrast`).

use super::Colors;
use gpui::Hsla;

/// The most opaque [`Colors::line_selection`] gets; above this One Dark's error text drops
/// under 4.5:1 on it.
const LINE_SELECTION_MAX_ALPHA: f32 = 0.17;

impl Colors {
    /// The fill of a selected line of text (a selected log line): [`Colors::selection`], capped
    /// at a strength that leaves level-coloured text readable. Drawn over
    /// [`Colors::background`].
    pub fn line_selection(&self) -> Hsla {
        Hsla {
            a: self.selection.a.min(LINE_SELECTION_MAX_ALPHA),
            ..self.selection
        }
    }

    /// The fill behind a search match inside a line of text: solid [`Colors::warning`]. Level
    /// colours cannot be read on a tint of it (the tint moves towards the text), so the matched
    /// text is repainted in [`Colors::search_match_text`] instead.
    pub fn search_match_fill(&self) -> Hsla {
        self.warning
    }

    /// The text colour of a search match: [`Colors::background`], which has the same contrast
    /// on [`Colors::search_match_fill`] as `warning` has on the background (at least 4.5:1).
    pub fn search_match_text(&self) -> Hsla {
        self.background
    }
}
