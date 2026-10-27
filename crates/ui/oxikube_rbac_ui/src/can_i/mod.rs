//! The can-i dialog (E17-S08): `rbac::CanI` (palette "RBAC: Can I?", prefilled from the selection)
//! asks whether you, or an impersonated User, Group or ServiceAccount, may do a verb on a
//! resource.
//!
//! | File | Holds |
//! |---|---|
//! | `dialog` | [`CanIDialog`]: the fields, the in-flight review (a new ask cancels the old one) and the result |
//! | `view` | drawing the dialog, with the copyable result |
//! | `memory` | [`SessionMemory`]: the last verb and subject, for the session only, never stored |
//! | `commands` | `rbac::CanI` on the bus ([`register_commands`], [`CanIRequestSink`]) and the window's [`CanIViews`] |
//!
//! The answer is `Allowed`, `Denied` or `Unknown`, with the API's reason and the name of the
//! review that gave it (`SelfSubjectAccessReview`, or `SubjectAccessReview` for another
//! subject). Reviews run on the Kubernetes runtime (`spawn_kube`) and are aborted when the dialog
//! asks again or closes. The impersonated subject lives in the dialog's inputs and the session
//! memory only.

mod commands;
mod dialog;
mod memory;
mod view;

pub use commands::{CanIOpen, CanIRequestSink, CanIViews, CanIViewsDeps, register_commands};
pub use dialog::{CanIDialog, CanIParts, CheckState, SubjectKind};
pub use memory::SessionMemory;
