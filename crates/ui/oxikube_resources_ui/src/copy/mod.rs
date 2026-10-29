//! Copy a resource to another cluster or namespace (E20-S08): "Copy to…" on any row.
//!
//! | File | Holds |
//! |---|---|
//! | `actions` | [`copy_row_actions`]: "Copy to…" on every kind's context menu and in the palette's list for the selection |
//! | `flow` | [`CopyFlow`]: opens the pickers and the dialog one after the other in the cluster tab's workspace |
//! | `cluster_picker` | [`ClusterPicker`]: the generic [`Picker`](oxikube_palette::Picker) (E11-S02) over the connected clusters, then the catalog's contexts |
//! | `namespace_picker` | [`NamespacePicker`]: the same picker over the target's namespaces (the cluster is connected on demand; a name that is not listed can be typed) |
//! | `dialog` | [`CopyDialog`]: the target's dry run, the choice about an existing object (overwrite, skip, rename), the typed name for a cluster-scoped kind, the run through [`CopyService`](oxikube_app::CopyService) |
//! | `view` | the dialog's rendering |
//! | `lines` | the review's sentences, pure |
//!
//! # How a user gets here
//!
//! Select a row of any kind, right-click it (or open the
//! palette's list for the selection) and choose **Copy to…** (`resource::CopyTo`). The detail
//! drawer's palette lists the same command. The flow asks, in order:
//!
//! 1. **the cluster**, connected ones first, then the other contexts of the kubeconfig (connected
//!    when chosen); a cluster known to be read-only is refused with a toast;
//! 2. **the namespace** (skipped for a cluster-scoped kind); one that does not exist can be typed,
//!    and the review offers to create it as its own guarded step;
//! 3. **the review**: the name of the copy (Enter reviews again), the target's server dry run (what
//!    changes), the fields the sanitiser dropped, references the target lacks, and, for an object
//!    that exists there, **Overwrite**, **Skip** or **Rename…** (`<name>-copy`). Overwriting over
//!    fields other managers own needs the extra "force" box; a cluster-scoped kind needs its
//!    name typed.
//!
//! Nothing is written before the last button, and the guard of the *target* cluster decides it:
//! read-only mode, the confirmation tier and the audit record are the target's.

mod actions;
mod cluster_picker;
mod dialog;
mod flow;
mod lines;
mod namespace_picker;
#[cfg(test)]
mod tests;
mod view;

pub use actions::{COPY_TO_ORDER, copy_row_actions};
pub use cluster_picker::{ClusterPicker, ClusterPickerDelegate};
pub use dialog::{Conflict, CopyDialog, CopyStage};
pub use flow::CopyFlow;
pub use lines::{MAX_REMOVED_SHOWN, change_line, error_line, removed_heading, rename_suggestion};
pub use namespace_picker::{NamespaceList, NamespacePicker, NamespacePickerDelegate};
