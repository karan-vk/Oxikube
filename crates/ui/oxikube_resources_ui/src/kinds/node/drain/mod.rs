//! The drain dialog (E12-S10): `kubectl drain` with its options, the plan before anything
//! changes, the node's name typed, per-pod progress, cancel and uncordon.
//!
//! | File | Holds |
//! |---|---|
//! | `dialog` | [`DrainDialog`]: reads the node's pods (off the UI thread), plans for the chosen flags, checks the numbers and the typed name |
//! | `run` | starts `node::Drain` with the typed name, follows its steps into the board, cancels by dropping the run, offers uncordon |
//! | `view` | drawing it: the switches (`--ignore-daemonsets`, `--delete-emptydir-data`, `--force`), grace period and timeout, the `kubectl drain` line, the rows, the buttons |
//! | `rows` | [`DrainLine`]s: the plan's pods (Blocks / Evict / Skip, with why) and the board's (Pending, Evicting, Blocked by PDB, Terminating, Deleted, Failed), drawn in a `uniform_list` |
//!
//! # How a user gets here
//!
//! On the Nodes table of a cluster tab (sidebar section Nodes): "Drain…" in a row's context menu,
//! the command palette ("Drain Node") with a node row under the cursor, the `r` key (k9s), or
//! the "Drain…" button of the node's detail panel. Each opens this dialog for one node.
//!
//! # Guard
//!
//! Nothing here calls a mutating port: Drain dispatches `node::Drain` on the bus, whose guard
//! refuses it on a read-only cluster, checks the name the user typed (risk high), and audits the
//! drain and each pod; "Cancel drain" drops the dispatch (the guard records it cancelled). The
//! steps come through the app's `DrainMonitor`, applied in batches with one coalesced notify, so
//! a busy drain costs one redraw per frame at most.

mod dialog;
mod rows;
mod run;
mod view;

pub use dialog::{DrainDialog, DrainLoaded, DrainStage, DrainSwitch};
pub use rows::{DrainLine, RowTone, plan_lines, progress_lines};

use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, WeakEntity, Window};
use oxikube_app::nodes::DrainFlow;
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::{CommandDispatcher, Workspace};

/// What opens the drain dialog from a detail (the node panel's button): the flow and the cluster
/// tab's workspace the dialog shows in. Cheap to clone.
#[derive(Clone)]
pub struct DrainOpener {
    flow: DrainFlow,
    workspace: WeakEntity<Workspace>,
}

impl DrainOpener {
    /// An opener over `flow`, showing the dialog in `workspace`.
    pub fn new(flow: DrainFlow, workspace: WeakEntity<Workspace>) -> Self {
        Self { flow, workspace }
    }

    /// Opens the dialog for `target`; see [`open_drain`].
    pub fn open(
        &self,
        dispatcher: Rc<dyn CommandDispatcher>,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<DrainDialog>> {
        open_drain(
            &self.workspace,
            self.flow.clone(),
            dispatcher,
            target,
            window,
            cx,
        )
    }
}

/// Opens the drain dialog for the node `target` as a modal of `workspace`. Does nothing without a
/// live workspace; returns the dialog so a caller (a test) can drive it.
pub fn open_drain(
    workspace: &WeakEntity<Workspace>,
    flow: DrainFlow,
    dispatcher: Rc<dyn CommandDispatcher>,
    target: ResourceRef,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<DrainDialog>> {
    let host = workspace.upgrade()?;
    let weak = workspace.clone();
    Some(host.update(cx, |workspace, cx| {
        let dialog = cx.new(|cx| DrainDialog::new(flow, target, weak, dispatcher, window, cx));
        workspace.show_modal(dialog.clone(), window, cx);
        dialog.update(cx, |dialog, cx| dialog.focus_name(window, cx));
        dialog
    }))
}
