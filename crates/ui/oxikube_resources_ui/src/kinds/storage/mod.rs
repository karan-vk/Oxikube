//! Storage (E12-S15): PersistentVolumeClaims, PersistentVolumes, StorageClasses and (where the
//! CSI snapshot CRDs are installed) VolumeSnapshots and VolumeSnapshotClasses.
//!
//! | File | Holds |
//! |---|---|
//! | `specs` | the five kinds' [`KindSpec`](crate::kinds::KindSpec)s: sidebar slot (Storage), detail panel, actions, and the claims' per-cluster columns |
//! | `usage` | the claims' "used by" feed, shared by the claims table and the claim detail of a cluster, and the claims' column provider |
//! | `panel` | the detail panels: `StoragePanel` over `PanelContent` |
//! | `dialogs` | [`ResizeDialog`](dialogs::ResizeDialog) and [`DefaultClassDialog`](dialogs::DefaultClassDialog) |
//! | `actions` | the row actions ("Resize…", "Set as Default") and the kind filters |
//! | `flows` | [`StorageFlows`], the flows the dialogs run, held by the table's actions |
//! | `open` | opening the dialogs in the cluster tab's workspace |
//!
//! # How a user gets here
//!
//! The Storage section of a cluster tab's sidebar lists Persistent Volume Claims, Persistent
//! Volumes and Storage Classes, and Volume Snapshots and Volume Snapshot Classes on clusters that
//! serve them (discovery gates the last two). **Columns**: claims show Status, Storage Class,
//! Size, Pods (used by) and the rest of kubectl's; volumes Capacity, Access Modes, Reclaim
//! Policy, Status, Claim; classes Provisioner, Reclaim Policy, Binding Mode, Allow Expansion and
//! a Default badge; snapshots Ready, Source, Class, Restore Size, Created. **Detail**: Enter on a
//! row opens the drawer with the kind's panel. **Actions**: on a claim row, "Resize…" in the
//! context menu, the command palette ("Resize Volume Claim") or `shift-r`; on a class row,
//! "Set as Default" in the menu, the palette or `shift-s`.
//!
//! # Guard
//!
//! Both actions are commands (`pvc::Resize`, risk medium; `storageclass::SetDefault`, risk low)
//! with MCP tool stubs (`k8s.pvc_resize`, `k8s.storageclass_set_default`). The dialogs only
//! collect arguments and show the same refusals the handler applies; the command goes through the
//! `MutationGuard` (read-only, the simple confirmation, the server's dry run, the audit record).
//!
//! # Performance
//!
//! The Pods column is a hash lookup per visible cell against an index maintained from the Pods
//! subscription off the UI thread (`oxikube_app::storage`). The feed exists only while a claims
//! table or a claim detail is open, and is shared between them.

mod actions;
pub mod dialogs;
mod flows;
pub(crate) mod open;
mod panel;
mod specs;
mod usage;

#[cfg(test)]
mod tests;

pub use actions::storage_row_actions;
pub use flows::StorageFlows;

use gpui::App;

use super::KindSpecs;

/// Registers the Storage section's kinds.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register_all(cx, specs::specs()) {
        tracing::error!(%error, "the storage kinds were not all registered");
    }
}
