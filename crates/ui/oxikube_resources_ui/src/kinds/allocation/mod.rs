//! The allocation section shared by the Node and Namespace panels (E13-S11): how full the node or
//! the namespace is, per resource, in requests, limits and live usage.
//!
//! | File | Holds |
//! |---|---|
//! | `section` | [`allocation_section`]: the rows of an [`AllocationView`](oxikube_app::allocation::AllocationView) drawn with the [`AllocationBar`](oxikube_ui::allocation_bar::AllocationBar) and the numbers beside it |
//! | `poll` | [`poll_usage`]: the task that asks the metrics port for usage while a panel is open |
//!
//! The view-model (the numbers, the text, the unavailable states) is `oxikube_app::allocation`;
//! nothing here computes an amount. A panel builds its [`AllocationView`] when its pods, its
//! object, its usage or the GPU vendor list change, keeps it, and hands it to [`allocation_section`]
//! on every frame.
//!
//! # Usage polling
//!
//! The usage segment needs `metrics.k8s.io`, a network call. [`poll_usage`] runs it on
//! `spawn_kube`, once when the panel opens and then every [`USAGE_INTERVAL`], and the panel keeps
//! the returned task: closing the panel drops it, which aborts the call and ends the polling, so
//! nothing polls for a detail nobody looks at. A reading that equals the last one does not wake
//! the panel. The cluster-wide polling service (E13-S03) will replace this source; the panels
//! only see a [`UsageState`](oxikube_app::allocation::UsageState).

mod poll;
mod section;

#[cfg(test)]
pub(crate) use poll::USAGE_INTERVAL;
pub(crate) use poll::{UsageSubject, UsageTarget, poll_usage};
pub(crate) use section::{allocation_section, muted};

#[cfg(test)]
mod tests;
