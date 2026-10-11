//! Scale (E12-S04): "Scale" on a Deployment's, StatefulSet's, ReplicaSet's or
//! ReplicationController's row and detail asks for the new replica count in a dialog and runs
//! `workload::Scale` through the guard.
//!
//! | File | Holds |
//! |---|---|
//! | `actions` | [`scale_row_actions`]: the row action (context menu, palette's list for the selection) |
//! | `flow` | [`ScaleFlow`]: opens the dialog in the cluster tab's workspace |
//! | `dialog` | [`ScaleDialog`]: reads the workload's current count, the field and its checks, and the run of `workload::Scale` through the [`ScaleRunner`](oxikube_app::ScaleRunner) |
//! | `view` | the dialog's rendering |
//!
//! # How a user gets here
//!
//! Right-click a Deployment, StatefulSet, ReplicaSet or ReplicationController row (or select it
//! and open the palette's list for the selection) and choose **Scale**, press `s` in the table
//! (k9s's key; `resource_table::ScaleSelected`, bound for those kinds only), or use the **Scale**
//! button in the detail panel. A DaemonSet has none of these: it cannot be scaled. A cluster in
//! read-only mode shows the action disabled with the reason; the guard refuses it for everyone.
//!
//! # What the dialog is
//!
//! It opens at once and reads the workload's `scale` subresource in the background (an input that
//! shows `Reading…` until then), so it appears within a frame whatever the network does. It shows
//! the current count (and the desired one when they differ: a rollout or an earlier scale in
//! flight), a field for the new count that must be a whole number of 0 or more, a warning when the
//! count is 0 (every pod stops) and a note when a HorizontalPodAutoscaler targets the workload (it
//! may set the count back). The button is the user's confirmation: it answers the guard's simple
//! confirmation once, and the command still passes the read-only check and the audit record. The
//! tier is the guard's: low, and medium for zero. The dialog never decides it.

mod actions;
mod dialog;
mod flow;
mod view;

pub use actions::{SCALE_ORDER, scale_row_actions};
pub use dialog::{ScaleDialog, ScaleStage};
pub use flow::ScaleFlow;
