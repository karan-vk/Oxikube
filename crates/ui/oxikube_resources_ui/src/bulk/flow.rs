//! [`BulkFlow`]: from a bulk action on a table's selection to the open dialog, and the row actions
//! that put the bulk commands in the menu.

use gpui::{App, AppContext as _, Entity, WeakEntity, Window};
use oxikube_app::actions::KindFilter;
use oxikube_app::{RowActionSpec, SelectionError, SelectionFlow};
use oxikube_domain::command::{Command, CommandId, Propagation};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::kinds::Verb;
use oxikube_workspace::Workspace;

use super::dialog::{BulkDialog, BulkSample};
use super::kind::BulkKind;

/// Where "Edit labels" sits in a menu: after the kind's own actions, before the generic delete.
const SELECTION_LABEL_ORDER: u16 = 880;

/// The row actions of the bulk commands: `selection::Label`, for any kind the server can patch,
/// one object or several ("Edit labels"). Delete and restart are the existing row actions that become the bulk ones when
/// several rows are selected ([`BulkKind::of_action`]); `selection::Delete` and
/// `selection::Restart` still appear in the palette through the table's own commands.
///
/// The command built here carries no edits and is never dispatched as it is: the dialog asks for
/// them and sends the command the user confirms.
pub fn bulk_row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::SELECTION_LABEL, |target| {
            Command::SelectionLabel {
                targets: vec![target.clone()],
                field: Default::default(),
                edits: Vec::new(),
            }
        })
        .label("Edit labels")
        .kinds(KindFilter::Verb(Verb::Patch))
        .bulk()
        .order(SELECTION_LABEL_ORDER),
    ]
}

/// Opens the bulk dialog in a cluster tab's workspace. Cheap to clone.
#[derive(Clone)]
pub struct BulkFlow {
    flow: SelectionFlow,
    workspace: WeakEntity<Workspace>,
}

impl BulkFlow {
    /// A flow that plans and runs through `flow` and asks in `workspace`.
    pub fn new(flow: SelectionFlow, workspace: WeakEntity<Workspace>) -> Self {
        Self { flow, workspace }
    }

    /// Opens the dialog for `kind` over `targets` at once. Does nothing without a live
    /// workspace. Returns the dialog so a caller can drive it.
    ///
    /// # Errors
    ///
    /// [`SelectionError::Empty`] / [`SelectionError::MixedClusters`] for a selection the guard
    /// would refuse; nothing opens.
    pub fn begin(
        &self,
        kind: BulkKind,
        targets: Vec<ResourceRef>,
        sample: BulkSample,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Option<Entity<BulkDialog>>, SelectionError> {
        let plan = self.flow.plan(BulkDialog::command_for(
            kind,
            targets,
            Propagation::Background,
            None,
        ))?;
        let Some(workspace) = self.workspace.upgrade() else {
            return Ok(None);
        };
        let (flow, weak) = (self.flow.clone(), self.workspace.clone());
        Ok(Some(workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| BulkDialog::new(kind, flow, plan, sample, weak, window, cx));
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus_input(window, cx));
            dialog
        })))
    }
}
