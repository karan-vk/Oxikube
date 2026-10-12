//! Rollout history and rollback (E12-S06): "Rollback" on a Deployment's, StatefulSet's,
//! DaemonSet's or ReplicaSet's row and detail opens the workload's revisions with their change
//! causes and, for the one chosen, what differs from the running template; its button runs
//! `workload::Rollback` through the guard.
//!
//! | File | Holds |
//! |---|---|
//! | `actions` | [`rollback_row_actions`]: the row action (context menu, the palette's list for the selection) |
//! | `flow` | [`RollbackFlow`]: opens the dialog in the cluster tab's workspace |
//! | `rows` | [`HistoryRow`]: a revision as a table row, computed once when the history arrives |
//! | `dialog` | [`RollbackDialog`]: reads the history, the selection and the run through the [`RollbackRunner`](oxikube_app::RollbackRunner) |
//! | `view` | the dialog's rendering: the history table, the preview, the confirmation line |
//!
//! # How a user gets here
//!
//! Select a Deployment, StatefulSet, DaemonSet or ReplicaSet row and press `ctrl-l` (k9s's
//! rollback key; `resource_table::RollbackSelected`), or right-click it (or open the palette's
//! list for the selection) and choose **Rollback**, or use the **Rollback** button in the workload
//! panel of the detail drawer. A ReplicaSet row rolls its Deployment back to that ReplicaSet's
//! revision. A cluster in read-only mode shows the action disabled with the reason; the guard
//! refuses it for everyone.
//!
//! # What the dialog is
//!
//! It opens at once and reads the history in the background (a list of the workload's
//! ReplicaSets or ControllerRevisions on the Tokio bridge; no feed is started), so it appears
//! within a frame whatever the network does. The table lists the revisions newest first: the
//! revision, its change cause, its images, how many pods its ReplicaSet runs and its age, with the
//! running one marked. The previous revision (or the ReplicaSet's own) starts selected; below the
//! table, the selected revision's differences from the running template (images, environment,
//! anything else that changed, capped) are the preview, and the line under it says the tier: a
//! rollback is **medium** risk and asks once. A paused Deployment is refused up front, with
//! kubectl's own sentence. The button is the user's confirmation: it answers the guard's simple
//! confirmation once, and the command still passes the read-only check and the audit record.

mod actions;
mod dialog;
mod flow;
mod rows;
mod view;

pub use actions::{ROLLBACK_ORDER, can_rollback, rollback_row_actions};
pub use dialog::{MAX_ROWS, RollbackDialog, RollbackStage};
pub use flow::RollbackFlow;
pub use rows::HistoryRow;
