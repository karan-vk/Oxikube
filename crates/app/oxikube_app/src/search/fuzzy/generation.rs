//! [`QueryGeneration`]: which asynchronous match result is still wanted.

/// A latest-wins counter for asynchronous match results: take [`Self::next`] when a query starts,
/// and write its result only while [`Self::is_current`] says it is still the newest. Two quick
/// keystrokes can both be matching on the background executor; only the second may write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryGeneration(u64);

impl QueryGeneration {
    /// Starts a new query and returns its generation.
    pub fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// The newest generation handed out.
    pub fn current(&self) -> u64 {
        self.0
    }

    /// Whether `generation` is still the newest query.
    pub fn is_current(&self, generation: u64) -> bool {
        self.0 == generation
    }
}
