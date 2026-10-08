//! The age tick (E07-F566): a detail view that is shown redraws for ages only when an age it
//! draws would read differently, not once a second.
//!
//! Ages are drawn by three views: the header's chip (every tab), the Overview's condition rows
//! and the Events tab's rows. Rather than a deadline per view, the tick re-reads them: it asks
//! whether any of the timestamps the active tab draws formats to different text at the current
//! time than at the time of the last frame. Most objects are days old and change once a day
//! (`30d`), so a still detail costs a timer wake-up and a handful of formatted strings, never a
//! frame. Same idea as the table's `CellCache::ages_moved`.

use std::time::Duration;

use jiff::Timestamp;
use oxikube_domain::Age;

use super::tabs::DetailTab;
use super::view::DetailView;

/// How often the view checks whether an age moved while it is shown.
pub(super) const TICK: Duration = Duration::from_secs(1);

/// Whether `at` reads differently as an age at `then` and at `now`.
fn moved(at: Timestamp, then: Timestamp, now: Timestamp) -> bool {
    Age::between(at, then).to_kubectl_string() != Age::between(at, now).to_kubectl_string()
}

impl DetailView {
    /// The creation, condition and event times the active tab draws as ages.
    fn drawn_ages(&self) -> impl Iterator<Item = Timestamp> + '_ {
        let model = self.model.as_ref();
        let created = model.and_then(|m| m.header.created);
        let conditions = model
            .filter(|_| self.tab == DetailTab::Overview)
            .into_iter()
            .flat_map(|m| m.conditions.iter().filter_map(|c| c.transition));
        let event_rows: &[_] = if self.tab == DetailTab::Events {
            &self.events.rows
        } else {
            &[]
        };
        let events = event_rows.iter().filter_map(|e| e.last_seen);
        created.into_iter().chain(conditions).chain(events)
    }

    /// Whether an age on screen reads differently at `now` than in the last frame.
    pub(super) fn ages_moved(&self, now: Timestamp) -> bool {
        self.drawn_ages().any(|at| moved(at, self.drawn_at, now))
    }

    /// One tick of the age timer: redraws if (and only if) the view is shown and an age moved.
    pub(super) fn tick_ages(&mut self, cx: &mut gpui::Context<Self>) {
        if self.shown && self.ages_moved(self.now()) {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_second(secs).unwrap()
    }

    #[test]
    fn an_age_moves_only_when_its_text_does() {
        let created = at(0);
        // 30 days old: "30d" for the whole day.
        let day30 = at(30 * 86_400);
        assert!(!moved(
            created,
            day30,
            day30 + jiff::SignedDuration::from_secs(60)
        ));
        assert!(moved(created, day30, at(31 * 86_400)));
        // Twenty seconds old: every second counts.
        assert!(moved(created, at(20), at(21)));
        assert!(!moved(created, at(20), at(20)));
    }
}
