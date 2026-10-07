//! [`LevelFilter`]: which levels a log view shows (the level chips).

use oxikube_domain::log::{LevelChip, LogLevel};

use super::entry::LogEntry;

/// The set of level chips that are on. Every chip is on by default (nothing is hidden); turning
/// one off hides the lines of that level: `trace` to `fatal` for structured lines, `text` for
/// plain-text lines and structured lines with no level.
///
/// It is one predicate over a line, [`admits`](Self::admits), so a view composes it with any other
/// line filter (the text search of E08-S03) in a single pass over the buffer. `Copy`, one byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelFilter {
    /// Bit `chip.index()` set means the chip is off (lines hidden), so the default `0` shows all.
    hidden: u8,
}

impl Default for LevelFilter {
    fn default() -> Self {
        Self::all()
    }
}

impl LevelFilter {
    /// Every chip on: no line is hidden.
    pub const fn all() -> Self {
        Self { hidden: 0 }
    }

    /// Whether every chip is on (the filter hides nothing).
    pub const fn is_all(self) -> bool {
        self.hidden == 0
    }

    /// Whether `chip` is on.
    pub const fn shows(self, chip: LevelChip) -> bool {
        self.hidden & (1 << chip.index()) == 0
    }

    /// Turns `chip` on or off.
    pub fn set(&mut self, chip: LevelChip, on: bool) {
        let bit = 1 << chip.index();
        if on {
            self.hidden &= !bit;
        } else {
            self.hidden |= bit;
        }
    }

    /// Flips `chip`; returns whether it is on now.
    pub fn toggle(&mut self, chip: LevelChip) -> bool {
        let on = !self.shows(chip);
        self.set(chip, on);
        on
    }

    /// Whether a line of `level` is shown: `None` is plain text (the `text` chip).
    const fn admits_level(self, level: Option<LogLevel>) -> bool {
        match level {
            Some(level) => self.shows(level.chip()),
            None => self.shows(LevelChip::Text),
        }
    }

    /// Whether `entry` is shown.
    pub const fn admits(self, entry: &LogEntry) -> bool {
        self.admits_level(entry.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_shown_by_default() {
        let filter = LevelFilter::default();
        assert!(filter.is_all());
        for chip in LevelChip::ALL {
            assert!(filter.shows(chip));
        }
        assert!(filter.admits_level(None));
        assert!(filter.admits_level(Some(LogLevel::Unknown)));
    }

    #[test]
    fn a_chip_toggles_its_own_lines_only() {
        let mut filter = LevelFilter::all();
        assert!(!filter.toggle(LevelChip::Debug));
        assert!(!filter.is_all());
        assert!(!filter.admits_level(Some(LogLevel::Debug)));
        assert!(filter.admits_level(Some(LogLevel::Info)));
        assert!(filter.admits_level(None), "plain text has its own chip");
        assert!(filter.toggle(LevelChip::Debug));
        assert!(filter.is_all());
    }

    #[test]
    fn text_covers_plain_lines_and_structured_lines_without_a_level() {
        let mut filter = LevelFilter::all();
        filter.set(LevelChip::Text, false);
        assert!(!filter.admits_level(None));
        assert!(!filter.admits_level(Some(LogLevel::Unknown)));
        assert!(filter.admits_level(Some(LogLevel::Error)));
    }

    #[test]
    fn all_seven_chips_fit_one_byte_and_are_independent() {
        let mut filter = LevelFilter::all();
        for chip in LevelChip::ALL {
            filter.set(chip, false);
        }
        for chip in LevelChip::ALL {
            assert!(!filter.shows(chip));
        }
        filter.set(LevelChip::Fatal, true);
        assert!(filter.shows(LevelChip::Fatal));
        assert!(!filter.shows(LevelChip::Text));
    }
}
