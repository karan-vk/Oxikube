//! [`LiveSync`]: what a buffer loaded from a live object does when the object changes on the
//! server. Pure state, driven by the view, tested without a window.
//!
//! ```text
//! Clean --edit--> Dirty --server change--> Banner --Merge--> Merging --accept--> Dirty
//! Clean --server change--> Clean (reloaded silently)          Banner --Reload--> Clean
//!                                                              Banner --Keep mine--> Dirty
//! ```
//!
//! Three versions are tracked: the **base** (the object the buffer was loaded from, or last
//! merged with), the **live** object (the latest the watch delivered) and the buffer (**mine**,
//! held by the editor). A change of the server-owned fields only (`status`, `resourceVersion`;
//! see [`IGNORED_FIELDS`](super::IGNORED_FIELDS)) never raises the banner over a dirty buffer,
//! nor resets a banner or a merge in progress.

use std::sync::Arc;

use oxikube_domain::Resource;

use super::Renderer;
use super::canonical::normalized_text;
use super::merge::{MergeSide, ThreeWayMerge};

/// Where a buffer stands against its live object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// The buffer holds the base unchanged: server changes reload it silently.
    Clean,
    /// The buffer was edited.
    Dirty,
    /// The object changed on the server while the buffer was dirty: Merge, Reload or Keep mine.
    Banner,
    /// A merge with conflicts waits for a side per conflict.
    Merging,
}

/// One version of the object, as the buffer and the diff see it. Built off the UI thread.
#[derive(Clone, Debug)]
pub struct ObjectVersion {
    /// The object.
    pub resource: Resource,
    /// What a buffer loaded from it holds (the renderer's text).
    pub text: Arc<str>,
    /// What counts as a change ([`normalized_text`]: server fields ignored, keys sorted).
    pub normalized: Arc<str>,
}

impl ObjectVersion {
    /// Renders `resource` with `render` and normalises it. Allocates the object twice over: call
    /// it on the background executor.
    pub fn new(resource: Resource, render: &Renderer) -> Self {
        let text = Arc::from(render(&resource));
        let normalized = Arc::from(normalized_text(&resource.to_value(), None));
        Self {
            resource,
            text,
            normalized,
        }
    }
}

/// What the watch says about the object.
#[derive(Clone, Debug)]
pub enum LiveObject {
    /// It exists, in this version.
    Present(Arc<ObjectVersion>),
    /// It was deleted (or never existed).
    Gone,
}

/// What the view does after [`LiveSync::live_changed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncAction {
    /// Nothing to do to the buffer.
    None,
    /// Replace the clean buffer with this text (silently).
    Reload(Arc<str>),
    /// Show the banner.
    Banner,
}

/// What [`LiveSync::merge`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeStep {
    /// Merged without conflicts: put this text in the buffer.
    Done(String),
    /// This many conflicts wait for a side.
    Conflicts(usize),
    /// Nothing to merge with (no base, or the object is gone).
    Unavailable,
}

/// A merge waiting for its conflicts, and the buffer version it was made from.
#[derive(Clone, Debug)]
struct PendingMerge {
    merge: ThreeWayMerge,
    server: Arc<ObjectVersion>,
}

/// The base / live / mine state machine of one editor. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct LiveSync {
    base: Option<Arc<ObjectVersion>>,
    live: Option<LiveObject>,
    state: SyncState,
    merge: Option<PendingMerge>,
}

impl Default for LiveSync {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveSync {
    /// A buffer not loaded from an object: the diff follows the live object, no banner.
    pub fn new() -> Self {
        Self {
            base: None,
            live: None,
            state: SyncState::Clean,
            merge: None,
        }
    }

    /// The buffer now holds `version`'s text (opened from the object, or reloaded).
    pub fn loaded(&mut self, version: Arc<ObjectVersion>) {
        self.live = Some(LiveObject::Present(version.clone()));
        self.base = Some(version);
        self.state = SyncState::Clean;
        self.merge = None;
    }

    /// Where the buffer stands.
    pub fn state(&self) -> SyncState {
        self.state
    }

    /// The object the buffer was loaded from (or last merged with), if it was.
    pub fn base(&self) -> Option<&Arc<ObjectVersion>> {
        self.base.as_ref()
    }

    /// The latest the watch delivered; `None` before its first event.
    pub fn live(&self) -> Option<&LiveObject> {
        self.live.as_ref()
    }

    /// The live object's `metadata.resourceVersion`, for a conditional replace (E10-S08).
    pub fn live_resource_version(&self) -> Option<&str> {
        match self.live.as_ref()? {
            LiveObject::Present(version) => version.resource.meta.resource_version.as_deref(),
            LiveObject::Gone => None,
        }
    }

    /// The merge waiting for sides, while [`SyncState::Merging`].
    pub fn pending_merge(&self) -> Option<&ThreeWayMerge> {
        self.merge.as_ref().map(|pending| &pending.merge)
    }

    /// The buffer changed; `dirty` says whether it now differs from what it was loaded with. A
    /// merge in progress is dropped (it was made from the old text): back to the banner.
    pub fn edited(&mut self, dirty: bool) {
        self.state = match self.state {
            SyncState::Clean | SyncState::Dirty if dirty => SyncState::Dirty,
            SyncState::Clean | SyncState::Dirty => SyncState::Clean,
            SyncState::Banner | SyncState::Merging => {
                self.merge = None;
                SyncState::Banner
            }
        };
    }

    /// The watch delivered `live`; `dirty` is whether the buffer was edited. See the module
    /// docs for what each state does.
    pub fn live_changed(&mut self, live: LiveObject, dirty: bool) -> SyncAction {
        let previous = self.live.replace(live.clone());
        let Some(base) = &self.base else {
            return SyncAction::None;
        };
        let version = match live {
            LiveObject::Present(version) => version,
            // Said once: a re-list that still finds nothing does not ask again.
            LiveObject::Gone if matches!(previous, Some(LiveObject::Gone)) => {
                return SyncAction::None;
            }
            LiveObject::Gone => return self.raise(),
        };
        if version.text == base.text {
            return SyncAction::None;
        }
        if self.state == SyncState::Clean && !dirty {
            self.base = Some(version.clone());
            return SyncAction::Reload(version.text.clone());
        }
        // The version the user is answering: the base while editing, the change the banner was
        // raised for, or the one the merge was made from. Only `status` / bookkeeping moved
        // since: not worth interrupting the edit, nor dropping the banner's merge or its picks.
        let answered = match self.state {
            SyncState::Clean | SyncState::Dirty => Some(base),
            SyncState::Banner => match &previous {
                Some(LiveObject::Present(raised_for)) => Some(raised_for),
                _ => None,
            },
            SyncState::Merging => self.merge.as_ref().map(|pending| &pending.server),
        };
        if answered.is_some_and(|answered| answered.normalized == version.normalized) {
            return SyncAction::None;
        }
        self.raise()
    }

    /// Whether `server` is still the live object, server-owned fields aside: a merge made from it
    /// (off the UI thread) still answers the server's change.
    pub fn is_current(&self, server: &ObjectVersion) -> bool {
        matches!(&self.live, Some(LiveObject::Present(live)) if live.normalized == server.normalized)
    }

    fn raise(&mut self) -> SyncAction {
        self.merge = None;
        self.state = SyncState::Banner;
        SyncAction::Banner
    }

    /// "Reload": the live object's text for the buffer, which is then clean. `None` when the
    /// object is gone or not known yet.
    pub fn reload(&mut self) -> Option<Arc<str>> {
        let Some(LiveObject::Present(version)) = self.live.clone() else {
            return None;
        };
        let text = version.text.clone();
        self.loaded(version);
        Some(text)
    }

    /// "Keep mine": the server's change is acknowledged (it is the new base) and the buffer
    /// stays as it is. Only while the banner is up (or a merge waits): without a server change
    /// to answer, a clean buffer must stay clean so the next change still reloads silently.
    pub fn keep_mine(&mut self) {
        if !matches!(self.state, SyncState::Banner | SyncState::Merging) {
            return;
        }
        if let Some(LiveObject::Present(version)) = &self.live {
            self.base = Some(version.clone());
        }
        self.merge = None;
        self.state = SyncState::Dirty;
    }

    /// "Merge": `mine` (the buffer's text) merged with the live object against the base. Runs
    /// the line merge here: the view computes it off the UI thread with [`merge_sides`] and
    /// hands it to [`merged`] instead.
    ///
    /// [`merge_sides`]: Self::merge_sides
    /// [`merged`]: Self::merged
    pub fn merge(&mut self, mine: &str) -> MergeStep {
        let Some((base, server)) = self.merge_sides() else {
            return MergeStep::Unavailable;
        };
        let merge = ThreeWayMerge::new(&base.text, mine, &server.text);
        self.merged(merge, server)
    }

    /// The base and the server's version a merge needs; `None` without a base or when the
    /// object is gone.
    pub fn merge_sides(&self) -> Option<(Arc<ObjectVersion>, Arc<ObjectVersion>)> {
        match (&self.base, &self.live) {
            (Some(base), Some(LiveObject::Present(server))) => Some((base.clone(), server.clone())),
            _ => None,
        }
    }

    /// Takes `merge` of the buffer with `server`: put its text in the buffer when it has no
    /// conflict, else wait for a side per conflict ([`SyncState::Merging`]).
    pub fn merged(&mut self, merge: ThreeWayMerge, server: Arc<ObjectVersion>) -> MergeStep {
        match merge.text() {
            Some(text) => {
                self.finish(server);
                MergeStep::Done(text)
            }
            None => {
                let conflicts = merge.conflicts();
                self.merge = Some(PendingMerge { merge, server });
                self.state = SyncState::Merging;
                MergeStep::Conflicts(conflicts)
            }
        }
    }

    /// Picks `side` for conflict `index` of the pending merge. Returns whether there was one.
    pub fn choose(&mut self, index: usize, side: MergeSide) -> bool {
        self.merge
            .as_mut()
            .is_some_and(|pending| pending.merge.choose(index, side))
    }

    /// Accepts the pending merge once every conflict has a side: its text for the buffer, which
    /// stays dirty against the server's version (the new base).
    pub fn accept(&mut self) -> Option<String> {
        let text = self.merge.as_ref()?.merge.text()?;
        let server = self.merge.take()?.server;
        self.finish(server);
        Some(text)
    }

    fn finish(&mut self, server: Arc<ObjectVersion>) {
        self.base = Some(server);
        self.merge = None;
        self.state = SyncState::Dirty;
    }
}
