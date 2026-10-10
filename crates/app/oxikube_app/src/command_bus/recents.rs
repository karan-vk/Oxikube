//! [`RecentsStore`]: the commands the user ran lately, most recent first, for the command
//! palette to list at the top.
//!
//! A trait so the store can change without touching the palette: [`MemoryRecents`] keeps a small
//! ring in memory (E11-S03: the fallback and the tests' store); `StateRecents`
//! (`search::recents`, E11-S11) keeps them through the state store so they survive a restart. Only
//! command ids are remembered: never arguments, targets or anything typed.

use std::collections::VecDeque;

use oxikube_domain::command::CommandId;
use parking_lot::Mutex;

/// How many recent commands are kept: [`MemoryRecents`] and, persisted, `StateRecents`.
pub const RECENTS_CAPACITY: usize = 50;

/// The commands run lately. Shared between windows, so `Send + Sync`; every call is quick and
/// in memory (an implementation that persists does so off the calling thread).
pub trait RecentsStore: Send + Sync {
    /// The recent commands, most recent first, without repeats.
    fn recent(&self) -> Vec<CommandId>;

    /// Notes that `id` has just been run: it becomes the most recent.
    fn record(&self, id: CommandId);

    /// Forgets every recent command (`palette::ClearRecents`).
    fn clear(&self);
}

/// A [`RecentsStore`] in memory: the last [`RECENTS_CAPACITY`] distinct commands.
#[derive(Debug, Default)]
pub struct MemoryRecents {
    ring: Mutex<VecDeque<CommandId>>,
}

impl MemoryRecents {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl RecentsStore for MemoryRecents {
    fn recent(&self) -> Vec<CommandId> {
        self.ring.lock().iter().copied().collect()
    }

    fn record(&self, id: CommandId) {
        let mut ring = self.ring.lock();
        ring.retain(|recent| *recent != id);
        ring.push_front(id);
        ring.truncate(RECENTS_CAPACITY);
    }

    fn clear(&self) {
        self.ring.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_run_comes_first_without_repeats() {
        let recents = MemoryRecents::new();
        assert!(recents.recent().is_empty());
        recents.record(CommandId::POD_DELETE);
        recents.record(CommandId::VIEW_ZOOM_IN);
        recents.record(CommandId::POD_DELETE);
        assert_eq!(
            recents.recent(),
            [CommandId::POD_DELETE, CommandId::VIEW_ZOOM_IN]
        );
    }

    #[test]
    fn clearing_forgets_everything() {
        let recents = MemoryRecents::new();
        recents.record(CommandId::POD_DELETE);
        recents.clear();
        assert!(recents.recent().is_empty());
    }

    #[test]
    fn the_ring_forgets_the_oldest() {
        let recents = MemoryRecents::new();
        for ix in 0..RECENTS_CAPACITY + 5 {
            recents.record(CommandId::new(Box::leak(
                format!("test::C{ix}").into_boxed_str(),
            )));
        }
        let recent = recents.recent();
        assert_eq!(recent.len(), RECENTS_CAPACITY);
        assert_eq!(
            recent[0].as_str(),
            format!("test::C{}", RECENTS_CAPACITY + 4)
        );
    }
}
