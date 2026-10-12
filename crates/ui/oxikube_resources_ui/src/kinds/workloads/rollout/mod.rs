//! Restart and set image (E12-S05): "Restart" and "Set image" on a Deployment's, StatefulSet's or
//! DaemonSet's row and detail run `workload::Restart` and `workload::SetImage` through the guard.
//!
//! | File | Holds |
//! |---|---|
//! | `actions` | [`rollout_row_actions`]: the two row actions (context menu, palette's list for the selection) |
//! | `flow` | [`SetImageFlow`]: opens the dialog in the cluster tab's workspace |
//! | `dialog` | [`SetImageDialog`]: reads the workload's containers, one field per container, the diff and the run of `workload::SetImage` through the [`SetImageRunner`](oxikube_app::SetImageRunner) |
//! | `view` | the dialog's rendering |
//!
//! # How a user gets here
//!
//! Right-click a Deployment, StatefulSet or DaemonSet row (or select it and open the palette's
//! list for the selection) and choose **Restart** or **Set image**, press `r` (restart) or `i`
//! (set image) in the table (k9s's keys; `resource_table::RestartSelected` and
//! `SetImageSelected`, bound for those kinds only), or use the two buttons in the detail
//! drawer's header. A ReplicaSet and a ReplicationController have none of these: their pods are
//! not rolled by a template change. A cluster in read-only mode shows the actions disabled with
//! the reason; the guard refuses them for everyone.
//!
//! # Restart
//!
//! Dispatched as the command it is: the bus asks its one confirmation ("Restart the workload?"),
//! the handler patches the pod template's `restartedAt` and the success toast carries an "Open"
//! button to the workload's detail, whose panel shows the rollout (Ready / Up-to-date /
//! Available) as the controller reports it. Nothing waits for the rollout.
//!
//! # The set-image dialog
//!
//! It opens at once and reads the workload in the background (`Reading the containers…`), so it
//! appears within a frame whatever the network does. Init containers come first, each with a field
//! holding the image it runs now. A field must not be empty or hold a space; the diff lists every
//! container whose field differs, old image against new, and the button (`Set N images`) is the
//! user's confirmation: it answers the guard's simple confirmation once and sends only the changed
//! containers. There is no registry lookup: the API server is the judge of the reference.

mod actions;
mod dialog;
mod flow;
mod view;

pub use actions::{RESTART_ORDER, SET_IMAGE_ORDER, rollout_row_actions};
pub use dialog::{ContainerField, SetImageDialog, SetImageStage};
pub use flow::SetImageFlow;
