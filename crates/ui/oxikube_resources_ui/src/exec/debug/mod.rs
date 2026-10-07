//! The debug-container dialog (E09-S10): "Debug" on a pod's row or detail asks for an image, a
//! target container, a command and an optional name, then adds the ephemeral container and opens a
//! terminal in it.
//!
//! | File | Holds |
//! |---|---|
//! | `flow` | `ExecFlow::begin_debug`: reads the pod (off the UI thread) for the dialog's defaults, then opens the dialog in the cluster tab's workspace |
//! | `dialog` | [`DebugDialog`]: the fields, their validation, and the run of `pod::Debug` through the [`DebugRunner`](oxikube_app::DebugRunner) |
//! | `view` | the dialog's rendering |
//!
//! # How a user gets here
//!
//! Right-click a Pod row (or select it and open the palette's list for the selection) and choose
//! **Debug**, press `shift-d` in a table, or use the bug button in a pod detail's header. The
//! dialog starts with `busybox` (or the image last used in this cluster), the pod's default
//! container as the target to share processes with, and `sh`. A cluster in read-only mode shows
//! the action disabled with the reason: the guard refuses it for everyone.
//!
//! # What the dialog is
//!
//! The dialog is the confirmation: it names the pod and says the container cannot be removed or
//! edited until the pod is deleted, and its button answers the guard's simple confirmation once
//! (see [`DebugRunner`](oxikube_app::DebugRunner)). The command still passes every guard stage.
//! While the container starts (an image pull can take a while) the dialog stays and shows
//! progress; a refusal (an admission policy, missing rights, an old server) comes back as the
//! API server's own message under the fields, so the user can fix the image and try again. On
//! success the dialog closes and the terminal opens in the bottom dock.

mod dialog;
mod flow;
mod view;

pub use dialog::{DebugDialog, DebugStage};
pub use view::PERMANENCE_NOTE;
