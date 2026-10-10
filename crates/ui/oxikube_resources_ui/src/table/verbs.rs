//! The k9s verbs of a table (E11-S07): `y`, `d`, `e`, `l`, `shift-f`, `f` and `ctrl-w`.
//!
//! Each is a view action that stands for a command, so the key, the context menu, the palette and
//! an agent run one behaviour (non-negotiable 4). The default bindings are in the per-OS keymap
//! files of `oxikube_assets`, in the `ResourceTable && !Editing` sections, so a letter typed
//! into the filter field is text.
//!
//! | Key | View action | Command | What happens |
//! |---|---|---|---|
//! | `y` | `ViewYaml` | `resource::ViewYaml` | the detail drawer opens on its YAML tab |
//! | `d` | `ViewDescribe` | `resource::ViewDescribe` | the detail drawer opens on its Describe tab |
//! | `e` | `EditSelected` | `resource::Edit` | the row action the editor registers; a toast says so until then |
//! | `l` | `ViewLogs` | `pod::ViewLogs`, `workload::ViewLogs` | the log viewer, for the pod or for the workload's pods |
//! | `shift-f` | `PortForward` | `pod::PortForward` | the row action port forwarding registers; a toast says so until then |
//! | `f` | `ShowPortForwards` | | a toast says so until port forwarding is installed |
//! | `ctrl-w` | `ToggleWide` | `table::ToggleWide` | the wide columns of every table of the kind show or hide |
//!
//! `ctrl-d` (delete), `s` (shell) and `a` (attach) are in [`row_actions`](super::row_actions).
//! The row-bound verbs act on the cursor row of a multi-selection and say so.

use gpui::{App, Context, Window};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_workspace::Toast;

use super::actions::{
    EditSelected, PortForward, ShowPortForwards, ToggleWide, ViewDescribe, ViewLogs, ViewYaml,
};
use super::view::ResourceTable;
use crate::exec::PORT_FORWARDING_UNAVAILABLE;

impl ResourceTable {
    /// `y`: dispatches `resource::ViewYaml` for the cursor row.
    pub(super) fn on_view_yaml(&mut self, _: &ViewYaml, _: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_for_cursor(|target| Command::ResourceViewYaml { target }, cx);
    }

    /// `d`: dispatches `resource::ViewDescribe` for the cursor row.
    pub(super) fn on_view_describe(
        &mut self,
        _: &ViewDescribe,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_for_cursor(|target| Command::ResourceViewDescribe { target }, cx);
    }

    /// `e`: the `resource::Edit` row action, which the manifest editor registers.
    pub(super) fn on_edit(
        &mut self,
        _: &EditSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::RESOURCE_EDIT, targets, window, cx);
    }

    /// `l`: the logs of the cursor row. A pod's own; any other kind asks for the workload logs
    /// (Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, Service), which the row actions of
    /// the log viewer offer only to those kinds.
    pub(super) fn on_view_logs(
        &mut self,
        _: &ViewLogs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = if self.kind.gvk.is_pod() {
            CommandId::POD_VIEW_LOGS
        } else {
            CommandId::WORKLOAD_VIEW_LOGS
        };
        let targets = self.action_targets(cx);
        self.run_action(command, targets, window, cx);
    }

    /// `shift-f`: the `pod::PortForward` row action, which port forwarding registers.
    pub(super) fn on_port_forward(
        &mut self,
        _: &PortForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_PORT_FORWARD, targets, window, cx);
    }

    /// `f`: the list of active port forwards, which port forwarding provides.
    pub(super) fn on_show_port_forwards(
        &mut self,
        _: &ShowPortForwards,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toast(
            Toast::info(PORT_FORWARDING_UNAVAILABLE).key("action-unavailable:port-forwards"),
            cx,
        );
    }

    /// `ctrl-w`: dispatches `table::ToggleWide`; the bus hands it back to
    /// [`ResourceViews`](crate::ResourceViews), which toggles every table of the kind.
    pub(super) fn on_toggle_wide(
        &mut self,
        _: &ToggleWide,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = Command::TableToggleWide {
            cluster: self.cluster.clone(),
            gvk: self.kind.gvk.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Shows the wide columns of this table, or hides them when they all show (what
    /// `table::ToggleWide` does). The choice is saved with the layout.
    pub fn toggle_wide(&mut self, cx: &mut Context<Self>) {
        if self.table.update(cx, |d| d.layout.toggle_wide()) {
            self.table.refresh(cx);
            // Hiding the sort column falls back to the default order.
            self.apply_sort(cx);
            self.save_prefs(cx);
            cx.notify();
        }
    }

    /// Dispatches `command(cursor row)`, or says there is no row.
    fn dispatch_for_cursor(
        &mut self,
        command: impl FnOnce(ResourceRef) -> Command,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.cursor_ref(cx) else {
            self.say_no_row(cx);
            return;
        };
        self.deps.dispatcher.dispatch(command(target), cx);
    }

    /// The object of the cursor row.
    fn cursor_ref(&self, cx: &App) -> Option<ResourceRef> {
        self.cursor_row(cx).and_then(|row| self.row_ref(row, cx))
    }
}
