//! Bulk actions on the table's selection (E12-S19): delete, restart, and edit labels or
//! annotations of several rows at once, with one confirmation and a result per row.
//!
//! The commands are `selection::Delete`, `selection::Restart` and `selection::Label` (one command
//! for labels and annotations, with a switch in the editor); this module is their front. Nothing
//! here calls a port: [`BulkDialog`] plans with [`SelectionFlow`](oxikube_app::SelectionFlow) and
//! runs it on the Tokio bridge, which sends the command through the `CommandBus` and the
//! `MutationGuard`.
//!
//! | File | Holds |
//! |---|---|
//! | `kind` | [`BulkKind`]: which bulk action a table action stands for, and when |
//! | `editor` | [`LabelEditor`]: the label editor's rows, the labels / annotations switch, the validated edits and the preview |
//! | `dialog` | [`BulkDialog`]: the modal's state and what the view reads |
//! | `run` | confirm, the run on the Tokio bridge, what comes back, the retry of the failed rows |
//! | `view` | the dialog's rendering: the question, the typed phrase, the progress, the results |
//! | `editor_view` | the label editor's rendering: the switch, the rows, the preview |
//! | `rows` | one result row of the virtualised results list |
//! | `flow` | [`BulkFlow`]: opens the dialog from the table; [`bulk_row_actions`] |
//!
//! # How a table reaches it
//!
//! * **Delete** (`Delete` key, `ctrl-d`, the menu, the palette's `Delete Selection`): with several
//!   rows selected the table opens this dialog instead of the single-object delete dialog. One
//!   row, and a Namespace (whose own `namespace::Delete` carries what it holds), keep the E07
//!   dialog.
//! * **Restart** (`r`, the menu, `Restart Selection`): with several Deployments, StatefulSets or
//!   DaemonSets selected. It is offered only when every selected row is one.
//! * **Edit labels** (`shift-l`, the menu, `Edit Labels of Selection`): any kind, one row or many.
//!
//! # Confirmation
//!
//! The dialog asks what the guard will ask, from the same policy: the highest tier among the
//! objects. A type-the-name tier (a Namespace in the selection, a cascading delete) asks for the
//! cluster's context name, not every object's name. The size of the selection never lowers it.
//! The results list is virtualised, so a selection of thousands stays cheap, and "Retry failed"
//! runs the same action over only the rows that failed.

mod dialog;
mod editor;
mod editor_view;
mod flow;
mod kind;
mod rows;
mod run;
mod view;

#[cfg(test)]
mod tests;

pub use dialog::{BulkDialog, BulkSample, BulkStage};
pub use editor::{EditorRow, LabelEditor};
pub use flow::{BulkFlow, SELECTION_LABEL_ORDER, bulk_row_actions};
pub use kind::BulkKind;
