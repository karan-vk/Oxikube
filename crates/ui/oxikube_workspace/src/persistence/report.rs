//! What a restore did, for the caller to show or log.

use crate::panel::DockPosition;

/// Why a saved item was not restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// No builder is registered for its kind (the feature crate is not loaded, or the kind was
    /// removed). Skipped, never fatal.
    UnknownKind,
    /// The kind's builder declined the saved state (the resource is gone, the format moved on).
    Declined,
    /// The item tab was saved without a kind and state.
    NoDescriptor,
}

/// An item that was in the saved layout and is not in the restored one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedItem {
    /// The saved kind (empty when [`SkipReason::NoDescriptor`]).
    pub kind: String,
    /// Why.
    pub reason: SkipReason,
}

/// The outcome of [`Workspace::restore_layout`](crate::Workspace::restore_layout).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// Items rebuilt and placed.
    pub restored_items: usize,
    /// Saved items that could not be rebuilt.
    pub skipped_items: Vec<SkippedItem>,
    /// The centre was replaced by the saved panes.
    pub centre_restored: bool,
    /// The centre was left as it was because items were already open (restore never closes the
    /// user's work).
    pub centre_kept: bool,
    /// The docks whose size, visibility and displayed panel were applied. A saved dock with no
    /// panel added to it yet is not in the list.
    pub docks_restored: Vec<DockPosition>,
}
