//! [`Follow`]: the autoscroll state of a log view with its "N new lines" count. Plain Rust, no gpui.

/// Autoscroll: on by default; scrolling up (or the autoscroll key) pauses it, and the view then
/// counts the lines that arrived since, by seq, for its "N new lines" pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Follow {
    on: bool,
    /// The window's `next_seq` when autoscroll paused.
    paused_at: u64,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            on: true,
            paused_at: 0,
        }
    }
}

impl Follow {
    /// Whether the view follows the newest line.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Stops following; lines from `next_seq` on count as new.
    pub fn pause(&mut self, next_seq: u64) {
        if self.on {
            self.on = false;
            self.paused_at = next_seq;
        }
    }

    /// Follows again (the pill goes).
    pub fn resume(&mut self) {
        self.on = true;
    }

    /// Lines that arrived since autoscroll paused, by seq: dropping old lines from the ring
    /// buffer does not change it. `0` while following.
    pub fn new_lines(&self, next_seq: u64) -> u64 {
        if self.on {
            0
        } else {
            next_seq.saturating_sub(self.paused_at)
        }
    }

    /// Starts over for a new stream (its seqs start at 0 again), keeping on or off.
    pub fn restart(&mut self) {
        self.paused_at = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_counts_by_seq_not_by_rows() {
        let mut follow = Follow::default();
        assert_eq!(follow.new_lines(100), 0);
        follow.pause(100);
        follow.pause(150); // already paused: the first point stays
        assert_eq!(follow.new_lines(130), 30);
        // The ring buffer dropped 1 000 old lines meanwhile: the count does not care.
        assert_eq!(follow.new_lines(1_300), 1_200);
        follow.resume();
        assert!(follow.is_on());
        assert_eq!(follow.new_lines(2_000), 0);
    }
}
