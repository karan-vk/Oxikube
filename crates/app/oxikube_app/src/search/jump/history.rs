//! [`JumpHistory`]: the jump lines run this session, with k9s's three moves through them.
//!
//! - `[` goes one line back, `]` one forward (like a browser's arrows);
//! - `-` goes to the view before the current one, and back again on a second use (k9s's "last
//!   view").
//!
//! A line run from the bar is [`record`](JumpHistory::record)ed after the current position and
//! drops the lines that were ahead of it. Running a line *because* of a move does not record it:
//! the move already put the position there. The ring itself is in memory; the lines of earlier runs
//! come from [`JumpRecents`](crate::JumpRecents) (E11-S11) through [`seed`](JumpHistory::seed), and
//! the bar records every confirmed line there.

/// How many lines are kept. The oldest go first.
pub const CAPACITY: usize = 100;

/// The lines run so far and where in them the user is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JumpHistory {
    lines: Vec<String>,
    /// The line shown now (an index into `lines`), `None` before the first jump.
    at: Option<usize>,
    /// The line shown before this one, for `-`.
    before: Option<usize>,
}

impl JumpHistory {
    /// An empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// How many lines are kept.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether nothing was run yet.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line shown now.
    pub fn current(&self) -> Option<&str> {
        self.at.and_then(|i| self.lines.get(i)).map(String::as_str)
    }

    /// Records `line` as the new current line. Running the line that is current already changes
    /// nothing; lines that were ahead of the position (after a `[`) are dropped.
    pub fn record(&mut self, line: &str) {
        if self.current() == Some(line) {
            return;
        }
        if let Some(at) = self.at {
            self.lines.truncate(at + 1);
        }
        self.before = self.at;
        self.lines.push(line.to_owned());
        if self.lines.len() > CAPACITY {
            self.lines.remove(0);
            self.before = self.before.and_then(|i| i.checked_sub(1));
        }
        self.at = Some(self.lines.len() - 1);
    }

    /// Puts the lines of earlier runs (`oldest_first`) behind what was run this session, so `[`
    /// reaches them. A ring that already has lines this run is left alone: the user moved on
    /// before the stored lines arrived.
    pub fn seed<'a>(&mut self, oldest_first: impl IntoIterator<Item = &'a str>) {
        if !self.is_empty() {
            return;
        }
        for line in oldest_first {
            self.record(line);
        }
    }

    /// `[`: moves one line back and returns it; `None` at the oldest line.
    pub fn back(&mut self) -> Option<&str> {
        let at = self.at.filter(|&i| i > 0)?;
        self.go(at - 1)
    }

    /// `]`: moves one line forward and returns it; `None` at the newest line.
    pub fn forward(&mut self) -> Option<&str> {
        let at = self.at.filter(|&i| i + 1 < self.lines.len())?;
        self.go(at + 1)
    }

    /// `-`: moves to the line shown before this one, which is where `-` goes back to again;
    /// `None` before a second line was shown.
    pub fn last(&mut self) -> Option<&str> {
        let before = self.before.filter(|&i| i < self.lines.len())?;
        self.go(before)
    }

    fn go(&mut self, to: usize) -> Option<&str> {
        self.before = self.at;
        self.at = Some(to);
        self.lines.get(to).map(String::as_str)
    }
}
