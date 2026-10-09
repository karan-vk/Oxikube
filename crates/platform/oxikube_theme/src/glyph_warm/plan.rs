//! [`DilationPlan`]: for each level a glyph is drawn at now, the levels a theme switch may draw it
//! at next.

use crate::tokens::ThemeTokens;
use gpui::Hsla;

/// How many dilation levels a plan can describe (GPUI's macOS text system uses 0 to 4; any other
/// platform draws every glyph at 0).
pub const LEVELS: u8 = 8;

/// For each glyph dilation level (the stroke thickening GPUI's macOS text system picks from the
/// luminance of the text colour, part of the glyph's atlas key), the set of levels the same glyph
/// is drawn at after a theme switch: a bit mask per level.
///
/// A glyph drawn now at level `from` gets warmed at every level in [`targets`](Self::targets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DilationPlan {
    targets: [u8; LEVELS as usize],
}

impl DilationPlan {
    /// The plan that warms nothing.
    pub const EMPTY: Self = Self {
        targets: [0; LEVELS as usize],
    };

    /// Records that a glyph drawn at level `from` may be drawn at level `to`. Same-level pairs
    /// and levels outside the plan are ignored.
    pub fn add(&mut self, from: u8, to: u8) {
        if from != to && from < LEVELS && to < LEVELS {
            self.targets[from as usize] |= 1 << to;
        }
    }

    /// The levels (bit mask) a glyph drawn at `from` is warmed at; empty outside the plan.
    pub fn targets(&self, from: u8) -> u8 {
        self.targets.get(from as usize).copied().unwrap_or(0)
    }

    /// The levels (bit mask) a glyph drawn at every level of the mask `drawn` is warmed at.
    pub fn targets_of(&self, drawn: u8) -> u8 {
        (0..LEVELS)
            .filter(|level| drawn & (1 << level) != 0)
            .fold(0, |acc, level| acc | self.targets(level))
    }

    /// Whether the plan warms nothing.
    pub fn is_empty(&self) -> bool {
        self.targets.iter().all(|t| *t == 0)
    }

    /// The plan for switching from `active` to any of `themes`: every colour slot of `active`
    /// paired with the same slot of each theme ([`ThemeTokens::for_each_color_pair`]), each colour
    /// mapped to its level by `dilation` (the platform's own `glyph_dilation_for_color`).
    ///
    /// Text drawn in a theme colour is drawn in the same slot's colour after the switch, so a
    /// glyph drawn now at the level of some slot is warmed at that slot's level in each theme.
    pub fn for_switch<'a>(
        active: &ThemeTokens,
        themes: impl IntoIterator<Item = &'a ThemeTokens>,
        dilation: impl Fn(Hsla) -> u8,
    ) -> Self {
        let mut plan = Self::EMPTY;
        for theme in themes {
            // Paired with itself every colour keeps its level: nothing to add.
            if theme.name == active.name {
                continue;
            }
            active.for_each_color_pair(theme, |from, to| plan.add(dilation(from), dilation(to)));
        }
        plan
    }
}
