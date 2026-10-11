//! Storage (E12-S15): the app side of the PVC, PV, StorageClass and VolumeSnapshot views.
//!
//! | File | Holds |
//! |---|---|
//! | `usage` | [`PvcUsage`], [`PvcUsageIndex`]: which Pods mount which claim, kept incrementally from the Pods feed |
//! | `feed` | [`PvcUsageFeed`]: the abort-on-drop task that feeds an index from a store, following the namespace selection |
//! | `columns` | [`PvcColumns`]: the claims table's columns, with the Pods (used by) column |
//! | `commands` | [`register_commands`]: the `pvc::Resize` and `storageclass::SetDefault` handlers |
//! | `flow` | [`ResizeFlow`], [`DefaultClassFlow`]: what the two dialogs read, check and send |
//!
//! # Guard tiers
//!
//! `pvc::Resize` is `Risk::Medium` (a simple confirm naming the claim and the cluster);
//! `storageclass::SetDefault` is `Risk::Low` (a simple confirm; ADR 0012's per-session skip of
//! low-risk confirms is E19's). Both are blocked on a read-only cluster, rehearsed with the
//! server's dry run, and audited: the resize's record carries the old and the new size, the
//! set-default's whether the previous default was cleared. No secret is read or written.
//!
//! # Performance
//!
//! The Pods column is one hash lookup per visible cell. The index is maintained off the UI thread
//! from the Pods subscription's deltas; a Pod update that mounts the same claims costs one walk
//! of its `spec.volumes` and no write lock. See [`usage`].

mod columns;
mod commands;
mod feed;
mod flow;
mod usage;

#[cfg(test)]
mod tests;

pub use columns::{PODS_COLUMN, PvcColumns};
pub use commands::register_commands;
pub use feed::PvcUsageFeed;
pub use flow::{
    DefaultClassContext, DefaultClassFlow, DefaultClassReport, ResizeContext, ResizeFlow,
    ResizeReport,
};
pub use usage::{PvcUsage, PvcUsageIndex, UsageStatus};
