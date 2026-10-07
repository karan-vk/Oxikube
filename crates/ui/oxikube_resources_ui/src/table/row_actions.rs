//! Row actions on a table (E07-S08): what the selection offers, running an action, and the
//! delete key.
//!
//! The list comes from `oxikube_app::actions` through [`ResourceActions`]; the menu, the
//! palette ([`ResourceTable::action_entries`]) and the delete key all end in
//! [`ResourceTable::run_action`], which opens the delete dialog for `resource::Delete` and sends
//! any other action's command through the table's dispatcher (the bus). Nothing here calls a
//! mutating port.

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::{Toast, Workspace};

use super::actions::{AttachSelected, DebugSelected, DeleteSelected, ShellSelected};
use super::view::ResourceTable;
use crate::actions::{ActionEntry, DeleteDialog, ResourceActions};
use crate::exec::{ExecFlow, ExecKind};

impl ResourceTable {
    /// Tells the table which workspace hosts its dialogs and toasts (its cluster tab's).
    pub fn set_workspace(&mut self, workspace: gpui::WeakEntity<Workspace>) {
        self.workspace = Some(workspace);
    }

    /// The objects an action acts on from the keyboard or the palette: the selected rows in row
    /// order, else the cursor row.
    pub fn action_targets(&self, cx: &gpui::App) -> Vec<ResourceRef> {
        let selected = self.selected_refs(cx);
        if !selected.is_empty() {
            return selected;
        }
        self.cursor_row(cx)
            .and_then(|row| self.row_ref(row, cx))
            .into_iter()
            .collect()
    }

    /// The actions the palette lists for the current selection: the list of the context menu
    /// (same registry, same session state), with a disabled action carrying its reason.
    pub fn action_entries(&self, cx: &gpui::App) -> Vec<ActionEntry> {
        let Some(actions) = &self.deps.actions else {
            return Vec::new();
        };
        actions.entries(&self.cluster, &self.kind, self.action_targets(cx).len())
    }

    /// Runs the action `command` on `targets` (objects of this table).
    ///
    /// A disabled action (read-only cluster) says why in a toast instead; an action this table
    /// does not offer is ignored. `resource::Delete` opens the [`DeleteDialog`]; another
    /// action's command is dispatched for each object, which is the bus's to guard and report.
    pub fn run_action(
        &mut self,
        command: CommandId,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(actions) = self.deps.actions.clone() else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        let entries = actions.entries(&self.cluster, &self.kind, targets.len());
        let Some(entry) = entries.into_iter().find(|entry| entry.command() == command) else {
            return;
        };
        if let Some(reason) = entry.reason() {
            self.toast(
                Toast::warning(reason).key(format!("action-disabled:{command}")),
                cx,
            );
            return;
        }
        if command == CommandId::RESOURCE_DELETE {
            self.begin_delete(&actions, targets, window, cx);
        } else if let Some(service) = actions
            .exec_service()
            .filter(|_| ExecKind::of(command).is_some() || command == CommandId::POD_DEBUG)
            .cloned()
        {
            // A shell, an attach or a debug container: the pod is read first, so the container is
            // chosen (or asked for, or the dialog filled in) before the command is dispatched and
            // audited.
            let workspace = self
                .workspace
                .clone()
                .unwrap_or_else(gpui::WeakEntity::new_invalid);
            let flow = ExecFlow::new(service, self.deps.dispatcher.clone(), workspace)
                .with_debug(actions.debug_runner());
            if let Some(target) = targets.into_iter().next() {
                self.exec_task = Some(match ExecKind::of(command) {
                    Some(kind) => flow.begin(kind, target, window, cx),
                    None => flow.begin_debug(target, window, cx),
                });
            }
        } else {
            for target in &targets {
                let command = entry.action.command_for(target);
                self.deps.dispatcher.dispatch(command, cx);
            }
        }
    }

    /// Opens the delete dialog for `targets`.
    fn begin_delete(
        &mut self,
        actions: &ResourceActions,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.as_ref().and_then(|ws| ws.upgrade()) else {
            tracing::warn!("a resource table without a workspace cannot open the delete dialog");
            return;
        };
        let plan = match actions.flow().plan(&targets, Default::default()) {
            Ok(plan) => plan,
            Err(error) => {
                self.toast(Toast::error(error.to_string()), cx);
                return;
            }
        };
        let flow = actions.flow().clone();
        let host = workspace.downgrade();
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| DeleteDialog::new(flow, plan, host, window, cx));
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus_input(window, cx));
        });
    }

    fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.as_ref() {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_toast(toast, cx);
                })
                .ok();
        }
    }

    pub(super) fn on_delete(
        &mut self,
        _: &DeleteSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::RESOURCE_DELETE, targets, window, cx);
    }

    pub(super) fn on_shell(
        &mut self,
        _: &ShellSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_SHELL, targets, window, cx);
    }

    pub(super) fn on_attach(
        &mut self,
        _: &AttachSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_ATTACH, targets, window, cx);
    }

    pub(super) fn on_debug(
        &mut self,
        _: &DebugSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_DEBUG, targets, window, cx);
    }
}
