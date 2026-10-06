//! [`ScriptedFeed`]: objects added, modified and deleted at chosen ticks, as the [`DeltaBatch`]es
//! a watch delivers.
//!
//! A UI or store test describes the life of a feed once (the initial list, then what changes at
//! tick 1, tick 2, ...) and gets the same batches the production reflector hands the store: one
//! [`Delta::Restarted`] with the initial objects at tick 0, then one batch per tick holding that
//! tick's `Applied` / `Deleted` deltas in the order they were scripted. [`install`] queues the
//! feed as the next watch of a [`FakeResourcePort`]; the test then moves the port's clock one
//! [`TICK`] at a time and each step delivers exactly one batch. Nothing sleeps and no thread
//! starts.
//!
//! ```
//! use oxikube_testkit::{FakeResourcePort, ScriptedFeed, pod};
//!
//! let feed = ScriptedFeed::new()
//!     .initial([pod().name("a").build()])
//!     .add(1, pod().name("b").build())
//!     .delete(2, pod().name("a").build());
//! assert_eq!(feed.last_tick(), 2);
//! assert_eq!(feed.batches().len(), 3);
//! feed.install(&FakeResourcePort::new());
//! ```
//!
//! [`install`]: ScriptedFeed::install

use std::collections::BTreeMap;
use std::time::Duration;

use oxikube_domain::Resource;
use oxikube_ports::{Delta, DeltaBatch};

use crate::{FakeResourcePort, Timeline};

/// How far the fake clock moves for one scripted tick.
pub const TICK: Duration = Duration::from_secs(1);

/// A scripted feed: the initial list at tick 0 and the changes of each later tick. See the
/// [module docs](self).
#[derive(Debug, Clone, Default)]
pub struct ScriptedFeed {
    initial: Vec<Resource>,
    steps: BTreeMap<u32, Vec<Delta>>,
}

impl ScriptedFeed {
    /// An empty feed: an empty initial list and no changes.
    pub fn new() -> Self {
        Self::default()
    }

    /// The objects the feed lists first (tick 0, one `Restarted` batch).
    #[must_use]
    pub fn initial(mut self, objects: impl IntoIterator<Item = Resource>) -> Self {
        self.initial = objects.into_iter().collect();
        self
    }

    /// `object` appears at `tick` (`tick >= 1`).
    #[must_use]
    pub fn add(self, tick: u32, object: Resource) -> Self {
        self.at(tick, [Delta::Applied(object)])
    }

    /// `object` changes at `tick`: its new state, as the API server's `MODIFIED` event.
    #[must_use]
    pub fn modify(self, tick: u32, object: Resource) -> Self {
        self.at(tick, [Delta::Applied(object)])
    }

    /// `object` (its last known state) is deleted at `tick`.
    #[must_use]
    pub fn delete(self, tick: u32, object: Resource) -> Self {
        self.at(tick, [Delta::Deleted(object)])
    }

    /// `deltas` all arrive at `tick`, in one batch and in this order.
    ///
    /// # Panics
    ///
    /// When `tick` is 0: the initial list is [`initial`](Self::initial).
    #[must_use]
    pub fn at(mut self, tick: u32, deltas: impl IntoIterator<Item = Delta>) -> Self {
        assert!(tick >= 1, "tick 0 is the initial list; use `initial`");
        self.steps.entry(tick).or_default().extend(deltas);
        self
    }

    /// The last tick that delivers something (0 when only the initial list is scripted).
    pub fn last_tick(&self) -> u32 {
        self.steps.keys().next_back().copied().unwrap_or(0)
    }

    /// Every batch with its tick, oldest first: tick 0 is the initial `Restarted`, then one
    /// batch per tick that changes something.
    pub fn batches(&self) -> Vec<(u32, DeltaBatch)> {
        let first = DeltaBatch::from_deltas(vec![Delta::Restarted(self.initial.clone())]);
        std::iter::once((0, first))
            .chain(
                self.steps
                    .iter()
                    .map(|(tick, deltas)| (*tick, DeltaBatch::from_deltas(deltas.clone()))),
            )
            .collect()
    }

    /// The batch delivered at `tick`, if any.
    pub fn batch_at(&self, tick: u32) -> Option<DeltaBatch> {
        self.batches()
            .into_iter()
            .find_map(|(t, batch)| (t == tick).then_some(batch))
    }

    /// The feed as a timeline: tick `n` at offset `n * TICK`, kept open like a live watch.
    pub fn timeline(&self) -> Timeline<DeltaBatch> {
        self.batches()
            .into_iter()
            .fold(Timeline::new(), |timeline, (tick, batch)| {
                timeline.ok_at(TICK * tick, batch)
            })
            .keep_open()
    }

    /// Queues the feed as the next watch `port` serves, and puts the initial objects in the
    /// port's store so `get`, `list` and `delete` agree with what the feed listed. The initial
    /// list arrives with the watch; move `port.clock()` by [`TICK`] to deliver the next tick
    /// (later ticks only reach the watch, not the store).
    pub fn install(&self, port: &FakeResourcePort) {
        for object in &self.initial {
            port.insert(object.clone());
        }
        port.script().watch.push_ok(self.timeline());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pod;

    fn named(name: &str) -> Resource {
        pod().namespace("x").name(name).build()
    }

    #[test]
    fn batches_are_the_initial_restart_then_one_batch_per_tick() {
        let feed = ScriptedFeed::new()
            .initial([named("a"), named("b")])
            .add(1, named("c"))
            .modify(1, named("a"))
            .delete(3, named("b"));
        assert_eq!(feed.last_tick(), 3);
        let batches = feed.batches();
        let ticks: Vec<u32> = batches.iter().map(|(t, _)| *t).collect();
        assert_eq!(ticks, [0, 1, 3], "tick 2 changes nothing and has no batch");
        assert!(matches!(
            batches[0].1.deltas.as_slice(),
            [Delta::Restarted(objects)] if objects.len() == 2
        ));
        // Same-tick changes stay in one batch, in scripted order.
        assert!(matches!(
            batches[1].1.deltas.as_slice(),
            [Delta::Applied(c), Delta::Applied(a)] if c.name() == "c" && a.name() == "a"
        ));
        assert!(feed.batch_at(2).is_none());
        assert!(matches!(
            feed.batch_at(3).unwrap().deltas.as_slice(),
            [Delta::Deleted(b)] if b.name() == "b"
        ));
    }

    #[test]
    fn an_empty_feed_lists_nothing_and_changes_nothing() {
        let feed = ScriptedFeed::new();
        assert_eq!(feed.last_tick(), 0);
        assert_eq!(feed.batches().len(), 1);
        assert!(feed.timeline().len() == 1);
    }

    #[test]
    #[should_panic(expected = "tick 0")]
    fn tick_zero_is_the_initial_list() {
        let _ = ScriptedFeed::new().add(0, named("a"));
    }
}
