//! [`NodeIndex`]: the cache's pods-by-node index and the states it passes through.
//!
//! The first build walks every cached pod (a few milliseconds at 10 000 pods), so it never runs on
//! the thread that asked for the index. The cache snapshots its objects ([`NodeBuild`]), the
//! requester builds from the snapshot on the store's spawner without holding the entry's lock,
//! and in the meantime the cache only notes which keys changed. Installing the result replays
//! those keys from the cache, which makes the index exactly what a build under the lock would
//! have produced.

use std::collections::HashSet;
use std::sync::Arc;

use super::PodsByNode;
use crate::store::object::{ObjectKey, StoreObject};

/// A snapshot of the cached pods to build the index from, tagged with the generation of the
/// request that took it (a result for an older generation is dropped).
#[derive(Debug)]
pub(crate) struct NodeBuild {
    pub generation: u64,
    pub objects: Vec<Arc<StoreObject>>,
}

/// The index, from off to ready.
#[derive(Debug, Default)]
pub(crate) enum NodeIndex {
    /// Nobody asked for it: pod deltas pay nothing.
    #[default]
    Off,
    /// A build from a snapshot is running; `journal` holds the keys that changed since.
    Building {
        generation: u64,
        journal: HashSet<ObjectKey>,
    },
    /// Built and kept in step per delta.
    Ready(PodsByNode),
}

impl NodeIndex {
    /// The index, once it is built.
    pub fn ready(&self) -> Option<&PodsByNode> {
        match self {
            Self::Ready(index) => Some(index),
            _ => None,
        }
    }

    /// Whether the build of `generation` is the one in progress.
    pub fn is_building(&self, generation: u64) -> bool {
        matches!(self, Self::Building { generation: running, .. } if *running == generation)
    }

    /// A pod was added or changed.
    pub fn note_upsert(&mut self, key: &ObjectKey, object: &Arc<StoreObject>) {
        match self {
            Self::Off => {}
            Self::Building { journal, .. } => {
                journal.insert(key.clone());
            }
            Self::Ready(index) => index.upsert(key, object),
        }
    }

    /// A pod was deleted.
    pub fn note_remove(&mut self, key: &ObjectKey) {
        match self {
            Self::Off => {}
            Self::Building { journal, .. } => {
                journal.insert(key.clone());
            }
            Self::Ready(index) => index.remove(key),
        }
    }

    /// Starts building, journalling from now on.
    pub fn begin(&mut self, generation: u64) {
        *self = Self::Building {
            generation,
            journal: HashSet::new(),
        };
    }

    /// Installs `built` if it answers the build in progress, after bringing it up to date with
    /// the keys that changed since its snapshot (`current` is the cache's own lookup). Anything
    /// else (the last lease left, or another one started a newer build) drops it.
    pub fn finish<'a>(
        &mut self,
        generation: u64,
        mut built: PodsByNode,
        current: impl Fn(&ObjectKey) -> Option<&'a Arc<StoreObject>>,
    ) {
        let Self::Building {
            generation: running,
            journal,
        } = self
        else {
            return;
        };
        if *running != generation {
            return;
        }
        for key in std::mem::take(journal) {
            match current(&key) {
                Some(object) => built.upsert(&key, object),
                None => built.remove(&key),
            }
        }
        *self = Self::Ready(built);
    }
}
