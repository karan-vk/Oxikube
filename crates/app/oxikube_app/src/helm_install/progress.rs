//! [`ReleaseProgress`]: who follows the output of which running release operation.
//!
//! The `helm::Install` handler runs the install inside its dispatch (the guard audits it as one
//! command); the tab that started it needs each line of `helm`'s output as it arrives. The handler
//! publishes every line here under the release, and followers subscribe before they dispatch. One
//! operation per release at a time: a second one is refused while the first runs. The upgrade tab
//! (E16-S10) follows its upgrades the same way.

use std::collections::HashMap;
use std::sync::Arc;

use futures::channel::mpsc;
use oxikube_domain::helm::{OutputLine, ReleaseRef};
use oxikube_domain::{OxiError, OxiResult};
use parking_lot::Mutex;

#[derive(Default)]
struct Entry {
    running: bool,
    followers: Vec<mpsc::UnboundedSender<OutputLine>>,
}

/// The running release operations and their followers. Cheap to clone; clones share the table.
#[derive(Clone, Default)]
pub struct ReleaseProgress {
    entries: Arc<Mutex<HashMap<ReleaseRef, Entry>>>,
}

impl std::fmt::Debug for ReleaseProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleaseProgress")
            .field("releases", &self.entries.lock().len())
            .finish()
    }
}

impl ReleaseProgress {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every output line of the next (or the running) operation on `release`, from now on. The
    /// receiver ends when that operation ends; drop it to stop following. Lines are not coalesced
    /// here: the view batches them per frame.
    pub fn subscribe(&self, release: &ReleaseRef) -> mpsc::UnboundedReceiver<OutputLine> {
        let (tx, rx) = mpsc::unbounded();
        let mut entries = self.entries.lock();
        let followers = &mut entries.entry(release.clone()).or_default().followers;
        followers.retain(|follower| !follower.is_closed());
        followers.push(tx);
        rx
    }

    /// Whether an operation on `release` is running.
    pub fn is_running(&self, release: &ReleaseRef) -> bool {
        self.entries
            .lock()
            .get(release)
            .is_some_and(|entry| entry.running)
    }

    /// Marks an operation on `release` as running and returns where its lines go.
    ///
    /// # Errors
    ///
    /// `Conflict` while another operation on the same release runs.
    pub(crate) fn start(&self, release: &ReleaseRef) -> OxiResult<ProgressPublisher> {
        let mut entries = self.entries.lock();
        let entry = entries.entry(release.clone()).or_default();
        if entry.running {
            return Err(OxiError::conflict(format!(
                "release {release} is being installed or upgraded already"
            )));
        }
        entry.running = true;
        Ok(ProgressPublisher {
            progress: self.clone(),
            release: release.clone(),
        })
    }

    /// Forgets the followers of a release whose operation never started (a refused dispatch), so
    /// a tab that gave up leaves nothing behind.
    pub fn forget(&self, release: &ReleaseRef) {
        let mut entries = self.entries.lock();
        if entries.get(release).is_some_and(|entry| !entry.running) {
            entries.remove(release);
        }
    }
}

/// The publishing end of one running operation. Dropping it ends the operation for its followers
/// (their receivers end) and lets the release be operated on again.
pub(crate) struct ProgressPublisher {
    progress: ReleaseProgress,
    release: ReleaseRef,
}

impl ProgressPublisher {
    /// Hands `line` to every follower; followers that went away are forgotten.
    pub(crate) fn publish(&self, line: &OutputLine) {
        let mut entries = self.progress.entries.lock();
        if let Some(entry) = entries.get_mut(&self.release) {
            entry
                .followers
                .retain(|follower| follower.unbounded_send(line.clone()).is_ok());
        }
    }
}

impl Drop for ProgressPublisher {
    fn drop(&mut self) {
        self.progress.entries.lock().remove(&self.release);
    }
}
