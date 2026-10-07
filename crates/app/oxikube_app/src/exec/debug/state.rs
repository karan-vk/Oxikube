//! [`DebugState`]: what the service remembers about debug containers.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::TerminalBackend;
use parking_lot::Mutex;

/// How many opened sessions wait for their terminal before the oldest is dropped (which ends it).
/// A terminal claims its session within a frame or two; more than a few waiting means the window
/// went away.
const MAX_UNCLAIMED: usize = 8;

/// An attached session waiting for its terminal tab.
struct Unclaimed {
    pod: ResourceRef,
    container: Arc<str>,
    backend: Box<dyn TerminalBackend>,
}

/// The last image per cluster (a convenience: the dialog starts with it) and the opened sessions
/// not yet claimed by a terminal.
#[derive(Default)]
pub(in crate::exec) struct DebugState {
    images: Mutex<HashMap<ClusterId, Arc<str>>>,
    unclaimed: Mutex<VecDeque<Unclaimed>>,
}

impl DebugState {
    /// The image last used for a debug container in `cluster` this session.
    pub(in crate::exec) fn last_image(&self, cluster: &ClusterId) -> Option<Arc<str>> {
        self.images.lock().get(cluster).cloned()
    }

    /// Records `image` as the last one used in `cluster`.
    pub(in crate::exec) fn remember_image(&self, cluster: &ClusterId, image: &str) {
        self.images.lock().insert(cluster.clone(), Arc::from(image));
    }

    /// Keeps `backend` until the terminal of `container` in `pod` claims it.
    pub(in crate::exec) fn hold(
        &self,
        pod: &ResourceRef,
        container: &str,
        backend: Box<dyn TerminalBackend>,
    ) {
        let mut waiting = self.unclaimed.lock();
        if waiting.len() >= MAX_UNCLAIMED {
            // Dropped here, which ends that session.
            waiting.pop_front();
        }
        waiting.push_back(Unclaimed {
            pod: pod.clone(),
            container: Arc::from(container),
            backend,
        });
    }

    /// The session held for `container` of `pod`, if there is one (once: a second claim, a
    /// reconnect, finds nothing and attaches anew).
    pub(in crate::exec) fn claim(
        &self,
        pod: &ResourceRef,
        container: &str,
    ) -> Option<Box<dyn TerminalBackend>> {
        let mut waiting = self.unclaimed.lock();
        let at = waiting
            .iter()
            .position(|held| held.pod == *pod && &*held.container == container)?;
        waiting.remove(at).map(|held| held.backend)
    }

    /// How many sessions wait for a terminal.
    pub(in crate::exec) fn unclaimed(&self) -> usize {
        self.unclaimed.lock().len()
    }
}
