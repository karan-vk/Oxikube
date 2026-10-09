//! The age tick (E07-F512, #605): the table redraws an age at the moment it reads differently,
//! and at no other time.
//!
//! After every frame the table looks up when the first cell it drew moves with the clock
//! ([`CellCache::next_move`](super::cell_cache::CellCache::next_move): an age reaching its next
//! second, minute or hour) and arms one timer for then, rounded up to the next whole second of the
//! wall clock. Kubernetes stamps times in whole seconds, so that is when ages change anyway; every
//! table (and both cluster tabs) wakes on the same boundary, so their redraws share one frame, and
//! no table wakes more than once a second. A screen of day-old objects wakes once an hour, not once
//! a second, and a screen without ages never.
//!
//! On waking, the table re-reads only the cells that moved ([`CellCache::refresh_moved`]) and
//! redraws only when one of them reads differently. The redraw then reuses every other cell's
//! text, and arms the next wake. A table that is not shown (another tab in front) does nothing on
//! waking and arms nothing until it is drawn again.
//!
//! Timers are detached and carry the generation they were armed in: a newer arm makes an older
//! timer a no-op instead of dropping it, so no task is cancelled from inside itself.
//!
//! [`CellCache::refresh_moved`]: super::cell_cache::CellCache::refresh_moved

use std::time::Duration;

use gpui::Context;
use jiff::Timestamp;

use super::view::ResourceTable;

/// The armed wake of a table's age tick.
#[derive(Debug, Default)]
pub(super) struct AgeTick {
    /// When the armed timer fires (`None`: nothing armed).
    armed: Option<Timestamp>,
    /// Bumped on every arm; a timer of an older generation does nothing.
    generation: u64,
    /// How many timers fired and found their generation current (tests).
    #[cfg(test)]
    pub(super) wakes: usize,
}

/// `at`, rounded up to a whole second of the wall clock.
pub(super) fn wake_at(at: Timestamp) -> Timestamp {
    if at.subsec_nanosecond() == 0 {
        return at;
    }
    Timestamp::from_second(at.as_second().saturating_add(1)).unwrap_or(at)
}

impl ResourceTable {
    /// Arms the timer for the first drawn cell that moves. Called after every frame of the table
    /// (deferred from render, so the frame's cells have been read). Keeps an armed timer that
    /// fires at the same second.
    pub(super) fn arm_age_tick(&mut self, cx: &mut Context<Self>) {
        let next = self.table.read(cx, |d| d.cells.next_move()).map(wake_at);
        if next == self.age_tick.armed {
            return;
        }
        self.age_tick.generation += 1;
        self.age_tick.armed = next;
        let Some(at) = next else {
            return;
        };
        let delay = Duration::try_from(at.duration_since(self.now())).unwrap_or_default();
        let generation = self.age_tick.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |view, cx| view.on_age_tick(generation, cx))
                .ok();
        })
        .detach();
    }

    /// A timer fired: re-read the cells that moved and redraw when one reads differently (the
    /// frame arms the next wake), else arm the next wake now.
    fn on_age_tick(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.age_tick.generation {
            return;
        }
        self.age_tick.armed = None;
        #[cfg(test)]
        {
            self.age_tick.wakes += 1;
        }
        if !self.active {
            // Not shown: its next frame arms it again.
            return;
        }
        let now = self.now();
        let moved = self
            .table
            .update_quiet(cx, |d| d.cells.refresh_moved(&*d.provider, now));
        if moved {
            cx.notify();
        } else {
            self.arm_age_tick(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakes_on_whole_seconds() {
        let s = Timestamp::from_second(1_800_000_000).unwrap();
        assert_eq!(wake_at(s), s);
        let later = s + jiff::SignedDuration::from_millis(1);
        assert_eq!(wake_at(later).as_second(), s.as_second() + 1);
        assert_eq!(wake_at(later).subsec_nanosecond(), 0);
    }
}
