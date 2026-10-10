//! Opening a shell or an attach from a pod's row or detail (E09-S08): the row actions, the
//! container picker, and the flow that joins them to the `pod::Shell` / `pod::Attach` commands.
//!
//! | File | Holds |
//! |---|---|
//! | `actions` | [`exec_row_actions`]: "Shell" and "Attach" on a Pod's context menu and in the palette's list for the selection, and "Shell" (`node::Shell`) on a Node |
//! | `flow` | [`ExecFlow`]: reads the pod, then either dispatches the command at once (one container, or the default) or opens the picker; [`ExecKind`] |
//! | `picker` | [`ContainerPicker`]: the modal that asks which container, the default (or the last choice for this pod) preselected; the generic picker (`oxikube_palette::Picker`, E11-S02) over a [`ContainerPickerDelegate`], so typing filters the containers |
//!
//! # How a user gets here
//!
//! Right-click a Pod row, or select it and open the palette's list for the selection: **Shell**
//! and **Attach**. The keys are `s` and `a` in a table (`resource_table::Shell`,
//! `resource_table::Attach`), the detail drawer's header has the same two buttons, and
//! `pod::Shell` / `pod::Attach` work from the palette and from agents. A pod with several
//! containers asks which one first; the terminal then opens in the bottom dock of the cluster's
//! tab ([`oxikube_terminal`'s `TerminalViews`]).
//!
//! A **Node** row offers **Shell** the same way (context menu, palette, `s`, and a button in the
//! detail header): it dispatches `node::Shell`, which confirms with the node and the image named
//! (a privileged pod is created on the node) and is greyed out on a read-only cluster.
//!
//! # What the picker is for
//!
//! The container is chosen *before* the command is dispatched, so the guard's audit record names
//! the container that really opened and cancelling the picker leaves no record of an open that
//! never happened. A command that names no container (the palette, an agent) opens the pod's
//! default container instead of asking.
//!
//! [`oxikube_terminal`'s `TerminalViews`]: https://github.com/karan-vk/Oxikube/blob/main/docs/ARCHITECTURE.md

mod actions;
mod debug;
mod flow;
mod picker;
#[cfg(test)]
mod tests;

pub use actions::{ATTACH_ORDER, DEBUG_ORDER, NODE_SHELL_ORDER, SHELL_ORDER, exec_row_actions};
pub(crate) use actions::{PORT_FORWARDING_UNAVAILABLE, acts_on_cursor_row, availability_hint};
pub use debug::{DebugDialog, DebugStage, PERMANENCE_NOTE};
pub use flow::{ExecFlow, ExecKind};
pub use picker::{ContainerPicker, ContainerPickerDelegate};
