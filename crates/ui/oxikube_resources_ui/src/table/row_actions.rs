//! Row actions on a table (E07-S08): what the selection offers, running an action, and the
//! delete key.
//!
//! The list comes from `oxikube_app::actions` through [`ResourceActions`]; the menu
//! ([`ResourceTable::action_entries`]) and the delete key all end in
//! [`ResourceTable::run_action`], which opens the delete dialog for `resource::Delete` and sends
//! any other action's command through the table's dispatcher (the bus). Nothing here calls a
//! mutating port.

use gpui::{AppContext as _, Context, Window};
use oxikube_app::CommandTarget;
use oxikube_app::namespaces::{delete_advice, delete_command};
use oxikube_domain::command::{self, CommandId, ViewContext};
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::command_surface::CommandSurface;
use oxikube_workspace::{Toast, Workspace};

use super::actions::{
    AttachSelected, CordonSelected, DebugSelected, DeleteSelected, RestartSelected,
    RollbackSelected, ScaleSelected, SetImageSelected, ShellSelected, UncordonSelected,
};
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

    /// The actions the context menu lists for the current selection (same registry, same session
    /// state), with a disabled action carrying its reason. The command palette (E11-S03) lists
    /// through `CommandBus::list` and this table's [`CommandSurface`] impl instead.
    pub fn action_entries(&self, cx: &gpui::App) -> Vec<ActionEntry> {
        let Some(actions) = &self.deps.actions else {
            return Vec::new();
        };
        actions.entries(&self.cluster, &self.kind, self.action_targets(cx).len())
    }

    /// Runs the action `command` on `targets` (objects of this table).
    ///
    /// A disabled action (read-only cluster) says why in a toast instead, and so does an action
    /// this table does not offer (E07-U563: "Shell is available for Pods and Nodes"). Shell,
    /// Attach and Debug take one object: of a selection they act on the cursor row and say so.
    /// `resource::Delete` opens the [`DeleteDialog`]; another action's command is dispatched for
    /// each object, which is the bus's to guard and report. Returns whether the action went ahead
    /// (a dialog opened or commands dispatched), `false` when it was refused with a toast.
    pub fn run_action(
        &mut self,
        command: CommandId,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(actions) = self.deps.actions.clone() else {
            return false;
        };
        // A Namespace is deleted by its own command (the generic delete does not offer it).
        let command = if command == CommandId::RESOURCE_DELETE && self.kind.gvk.is_namespace() {
            CommandId::NAMESPACE_DELETE
        } else {
            command
        };
        // Creating needs no row, and no Namespaces table: it asks for a name in its own dialog,
        // wherever the palette was opened.
        if command == CommandId::NAMESPACE_CREATE {
            self.open_create_namespace(window, cx);
            return true;
        }
        // A pod's delete is `pod::Delete`: the key, the palette's `resource::Delete` and the menu
        // all open the one delete dialog, which plans the pod's own command (E12-S03).
        let command = if command == CommandId::RESOURCE_DELETE && self.kind.gvk.is_pod() {
            CommandId::POD_DELETE
        } else {
            command
        };
        // A sweep of the view needs no row (an empty selection is the usual case).
        let targets = if command == CommandId::POD_SANITIZE && targets.is_empty() {
            self.cursor_row(cx)
                .and_then(|row| self.row_ref(row, cx))
                .into_iter()
                .collect()
        } else {
            targets
        };
        if targets.is_empty() && command != CommandId::POD_SANITIZE {
            self.say_no_row(cx);
            return false;
        }
        // Shell, attach and debug take one object: a selection acts on the cursor row.
        let selected = targets.len();
        let cursor_target = self.cursor_target(command, &targets, cx);
        let targets = cursor_target.clone().map_or(targets, |target| vec![target]);
        let entries = actions.entries(&self.cluster, &self.kind, targets.len());
        let Some(entry) = entries.into_iter().find(|entry| entry.command() == command) else {
            self.say_unavailable(&actions, command, selected, cx);
            return false;
        };
        if let Some(reason) = entry.reason() {
            self.toast(
                Toast::warning(reason).key(format!("action-disabled:{command}")),
                cx,
            );
            return false;
        }
        if let Some(target) = &cursor_target {
            self.say_cursor_row(command, entry.action.label(), target, selected, cx);
        }
        if command == CommandId::POD_SANITIZE {
            // A sweep acts on the view, whatever is selected.
            self.sanitize_view(cx);
        } else if matches!(
            command,
            CommandId::RESOURCE_DELETE | CommandId::POD_DELETE | CommandId::NAMESPACE_DELETE
        ) {
            self.begin_delete(&actions, targets, window, cx);
        } else if command == CommandId::WORKLOAD_SCALE {
            // The dialog asks for the count; the command is sent when the user confirms it.
            if let Some(target) = targets.into_iter().next() {
                let workspace = self
                    .workspace
                    .clone()
                    .unwrap_or_else(gpui::WeakEntity::new_invalid);
                actions.scale_flow(workspace).begin(target, window, cx);
            }
        } else if matches!(
            command,
            CommandId::PVC_RESIZE | CommandId::STORAGECLASS_SET_DEFAULT
        ) {
            self.begin_storage_dialog(&actions, command, targets, window, cx);
        } else if command == CommandId::WORKLOAD_SET_IMAGE {
            // The dialog asks for the images; the command is sent when the user confirms it.
            if let Some(target) = targets.into_iter().next() {
                let workspace = self
                    .workspace
                    .clone()
                    .unwrap_or_else(gpui::WeakEntity::new_invalid);
                actions.set_image_flow(workspace).begin(target, window, cx);
            }
        } else if command == CommandId::WORKLOAD_ROLLBACK {
            // The dialog shows the history and asks for the revision; the command is sent when
            // the user confirms it.
            if let Some(target) = targets.into_iter().next() {
                let workspace = self
                    .workspace
                    .clone()
                    .unwrap_or_else(gpui::WeakEntity::new_invalid);
                actions.rollback_flow(workspace).begin(target, window, cx);
            }
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
        true
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
        // A Namespace is deleted by `namespace::Delete`, carrying what it holds (counted from the
        // caches the app already has) and warned about before the name is typed.
        let mut flow = actions.flow().clone();
        let mut notes = Vec::new();
        if self.kind.gvk.is_namespace() {
            let several = targets.len() > 1;
            let mut contents = std::collections::HashMap::new();
            for target in &targets {
                let advice = delete_advice(self.store.as_ref(), &target.name);
                notes.extend(advice.notes.into_iter().map(|mut note| {
                    if several {
                        note.text = format!("{}: {}", target.name, note.text);
                    }
                    note
                }));
                contents.insert(target.name.to_string(), advice.contents);
            }
            notes.sort_by_key(|note| std::cmp::Reverse(note.level));
            flow = flow.with_command(delete_command(contents));
        }
        let plan = match flow.plan(&targets, Default::default()) {
            Ok(plan) => plan,
            Err(error) => {
                self.toast(Toast::error(error.to_string()), cx);
                return;
            }
        };
        let host = workspace.downgrade();
        workspace.update(cx, |workspace, cx| {
            let dialog =
                cx.new(|cx| DeleteDialog::new(flow, plan, host, window, cx).with_notes(notes));
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus_input(window, cx));
        });
    }

    pub(super) fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
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
        // `run_action` turns this into `namespace::Delete` on the Namespaces table.
        self.run_action(CommandId::RESOURCE_DELETE, targets, window, cx);
    }

    pub(super) fn on_shell(
        &mut self,
        _: &ShellSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        // On a node table the same key opens a node shell (`node::Shell`: confirmed, audited).
        let command = if self.kind.gvk.is_node() {
            CommandId::NODE_SHELL
        } else {
            CommandId::POD_SHELL
        };
        self.run_action(command, targets, window, cx);
    }

    /// `s` on a workload: the scale dialog for the cursor row (`workload::Scale`; k9s's scale key).
    pub(super) fn on_scale(
        &mut self,
        _: &ScaleSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::WORKLOAD_SCALE, targets, window, cx);
    }

    /// `r` on a Deployment, StatefulSet or DaemonSet: `workload::Restart` for the cursor row (k9s's
    /// restart key; the bus asks its one confirmation).
    pub(super) fn on_restart(
        &mut self,
        _: &RestartSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::WORKLOAD_RESTART, targets, window, cx);
    }

    /// `i` on a Deployment, StatefulSet or DaemonSet: the set-image dialog for the cursor row
    /// (`workload::SetImage`; k9s's set-image key).
    pub(super) fn on_set_image(
        &mut self,
        _: &SetImageSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::WORKLOAD_SET_IMAGE, targets, window, cx);
    }

    /// `ctrl-l` on a workload: the rollback dialog for the cursor row (`workload::Rollback`;
    /// k9s's rollback key).
    pub(super) fn on_rollback(
        &mut self,
        _: &RollbackSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::WORKLOAD_ROLLBACK, targets, window, cx);
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

    pub(super) fn on_cordon(
        &mut self,
        _: &CordonSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::NODE_CORDON, targets, window, cx);
    }

    pub(super) fn on_uncordon(
        &mut self,
        _: &UncordonSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::NODE_UNCORDON, targets, window, cx);
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

/// The command palette (E11-S03) reads a focused table as a `Table` view acting on its selected
/// rows, else its cursor row: the objects its own row actions and keys act on. It runs the table's
/// row actions through [`ResourceTable::run_action`], so a palette `Delete` opens the one delete
/// dialog for the whole selection (plan, propagation, type-the-name) as the key and the menu do,
/// and Shell, Attach and Debug choose their container first.
impl CommandSurface for ResourceTable {
    fn view_context(&self) -> ViewContext {
        ViewContext::Table
    }

    fn command_target(&self, cx: &gpui::App) -> CommandTarget {
        CommandTarget::none()
            .in_cluster(self.cluster.clone())
            .of_kind(self.kind.gvk.clone())
            .selecting(self.action_targets(cx))
    }

    fn own_commands(&self, cx: &gpui::App) -> Vec<CommandId> {
        let mut own: Vec<CommandId> = self
            .action_entries(cx)
            .into_iter()
            .filter(|entry| entry.reason().is_none())
            .map(|entry| entry.command())
            .collect();
        // The create dialog opens over any table: the palette lists `namespace::Create` from a
        // Pods table too, and only this flow can ask for the name.
        if let Some(actions) = &self.deps.actions
            && !own.contains(&CommandId::NAMESPACE_CREATE)
            && command::lookup(CommandId::NAMESPACE_CREATE).is_some_and(|meta| {
                actions
                    .context(&self.cluster)
                    .state_of(meta)
                    .reason()
                    .is_none()
            })
        {
            own.push(CommandId::NAMESPACE_CREATE);
        }
        // A pod's delete is `pod::Delete`, and `resource::Delete` is the same dialog for it: the
        // palette hands either to this table (E12-S03).
        if own.contains(&CommandId::POD_DELETE) {
            own.push(CommandId::RESOURCE_DELETE);
        }
        own
    }

    fn run_command(
        &mut self,
        command: CommandId,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.run_action(command, targets, window, cx);
        true
    }
}
