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
//! | `e` | `EditSelected` | `resource::Edit` | the row action the manifest editor registers (E10-S06): an editable `kind/name` tab; a toast says so when the editor is not installed |
//! | `l` | `ViewLogs` | `pod::ViewLogs`, `workload::ViewLogs` | the log viewer, for the pod or for the workload's pods |
//! | `shift-f` | `PortForward` | `pod::PortForward` | the start dialog for the row's pod (a Service row: `portforward::Start`); it sends `portforward::Start` on submit (E15-S04) |
//! | `f` | `ShowPortForwards` | `portforward::TogglePanel` | the Port Forwards panel opens in the bottom dock, listing the forwards of every cluster (E15-S05) |
//! | `ctrl-w` | `ToggleWide` | `table::ToggleWide` | the wide columns of every table of the kind show or hide |
//! | `u` (Services) | `ShowServicePods` | `service::ShowPods` | the Pods table, filtered by the Service's selector (E12-S13) |
//! | `o` (Services) | `OpenInBrowser` | `service::OpenInBrowser` | a port of the Service in the browser, once port forwarding is installed (E12-S13) |
//! | `u` (SA, ConfigMap, Secret, PVC, PriorityClass) | `ShowUsedBy` | `resource::ShowUsedBy` | the detail drawer opens on its "Used by" tab: the workloads whose Pods name the object (E12-S18) |
//! | `x` | `RevealSelected` | `secret::Reveal` | a Secret's detail opens with its values decoded, in memory (E12-S11) |
//! | `shift-e` | `EditDecodedSelected` | `secret::EditDecoded` | a Secret in the editor with its values as text (E12-S11) |
//!
//! `ctrl-d` (delete), `s` (shell) and `a` (attach) are in [`row_actions`](super::row_actions).
//! The row-bound verbs act on the cursor row of a multi-selection and say so.

use gpui::{App, Context, Window};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;

use super::actions::{
    EditDecodedSelected, EditSelected, OpenInBrowser, PortForward, RevealSelected,
    ShowPortForwards, ShowServicePods, ShowUsedBy, ToggleWide, ViewDescribe, ViewLogs, ViewYaml,
};
use super::view::ResourceTable;

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

    /// `x`: the `secret::Reveal` row action of the Secrets kind (E12-S11).
    pub(super) fn on_reveal(
        &mut self,
        _: &RevealSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::SECRET_REVEAL, targets, window, cx);
    }

    /// `shift-e`: the `secret::EditDecoded` row action of the Secrets kind (E12-S11).
    pub(super) fn on_edit_decoded(
        &mut self,
        _: &EditDecodedSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::SECRET_EDIT_DECODED, targets, window, cx);
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

    /// `shift-f`: the start dialog for the cursor row. A pod's row action is `pod::PortForward`; a
    /// Service's is `portforward::Start`. The dialog sends `portforward::Start` when submitted.
    pub(super) fn on_port_forward(
        &mut self,
        _: &PortForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = if self.kind.gvk.is_service() {
            CommandId::PORTFORWARD_START
        } else {
            CommandId::POD_PORT_FORWARD
        };
        let targets = self.action_targets(cx);
        self.run_action(command, targets, window, cx);
    }

    /// `u` on a Service: the `service::ShowPods` row action, the Pods table filtered by the
    /// Service's selector.
    pub(super) fn on_show_service_pods(
        &mut self,
        _: &ShowServicePods,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::SERVICE_SHOW_PODS, targets, window, cx);
    }

    /// `u` on a ServiceAccount, ConfigMap, Secret, claim or PriorityClass: the
    /// `resource::ShowUsedBy` row action, the detail on its "Used by" tab.
    pub(super) fn on_show_used_by(
        &mut self,
        _: &ShowUsedBy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::RESOURCE_SHOW_USED_BY, targets, window, cx);
    }

    /// `o` on a Service: the `service::OpenInBrowser` row action, which a port-forward manager
    /// registers; a toast says so until then.
    pub(super) fn on_open_in_browser(
        &mut self,
        _: &OpenInBrowser,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::SERVICE_OPEN_IN_BROWSER, targets, window, cx);
    }

    /// `f`: shows the Port Forwards panel, which lists every cluster's forwards
    /// (`portforward::TogglePanel`, shown rather than toggled).
    pub(super) fn on_show_port_forwards(
        &mut self,
        _: &ShowPortForwards,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.deps
            .dispatcher
            .dispatch(Command::PortForwardTogglePanel { show: true }, cx);
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
