//! The Namespaces table's keys and create dialog (E12-S16): `u` uses the namespace under the
//! cursor, `p` pins it, `0`-`9` select All or the nth favourite (in any table), and the **New**
//! button opens the create dialog.
//!
//! Each key is a view action that stands for a command (`namespace::Select`,
//! `namespace::ToggleFavourite`, `namespace::UseFavourite`, `namespace::Create`), so the key, the
//! context menu, the palette and an agent run one behaviour. Using a namespace and pinning one
//! change what the session watches and a local list, so they work on a read-only cluster; the
//! create dialog is a mutation and says so when the cluster is read-only.

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_workspace::Toast;

use super::actions::{SelectNamespaceSlot, ToggleFavouriteNamespace, UseNamespace};
use super::view::ResourceTable;
use crate::kinds::namespace::CreateNamespaceDialog;

impl ResourceTable {
    /// `u`: uses the namespace under the cursor.
    pub(super) fn on_use_namespace(
        &mut self,
        _: &UseNamespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The Namespaces list is cluster-wide, so nothing in it narrows: say what happened.
        self.run_namespace_action(
            CommandId::NAMESPACE_SELECT,
            "Watching namespace",
            "namespace-use",
            window,
            cx,
        );
    }

    /// `p`: pins or unpins the namespace under the cursor.
    pub(super) fn on_toggle_favourite_namespace(
        &mut self,
        _: &ToggleFavouriteNamespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_namespace_action(
            CommandId::NAMESPACE_TOGGLE_FAVOURITE,
            "Toggled favourite",
            "namespace-favourite",
            window,
            cx,
        );
    }

    /// Runs `command`, then toasts `"{said} {name}"` on the Namespaces table, naming the row the
    /// command ran on (the cursor row of a selection) and only when it went ahead.
    fn run_namespace_action(
        &mut self,
        command: CommandId,
        said: &str,
        toast_key: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        // The row `run_action` narrows a selection to, else the only target.
        let name = self
            .cursor_target(command, &targets, cx)
            .or_else(|| targets.first().cloned())
            .map(|t| t.name.to_string());
        let ran = self.run_action(command, targets, window, cx);
        if let Some(name) = name.filter(|_| ran && self.kind.gvk.is_namespace()) {
            self.toast(Toast::info(format!("{said} {name}")).key(toast_key), cx);
        }
    }

    /// `0`-`9`: dispatches `namespace::UseFavourite` for this table's cluster.
    pub(super) fn on_select_namespace_slot(
        &mut self,
        action: &SelectNamespaceSlot,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = Command::NamespaceUseFavourite {
            cluster: self.cluster.clone(),
            slot: action.slot,
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Opens the create-namespace dialog for this table's cluster. A read-only cluster says why
    /// in a toast instead (creating is a mutation; the guard would refuse it anyway).
    pub fn open_create_namespace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(actions), Some(workspace)) = (
            self.deps.actions.clone(),
            self.workspace.as_ref().and_then(|ws| ws.upgrade()),
        ) else {
            tracing::warn!("a resource table without a workspace cannot open the create dialog");
            return;
        };
        if let Some(meta) = command::lookup(CommandId::NAMESPACE_CREATE)
            && let Some(reason) = actions.context(&self.cluster).state_of(meta).reason()
        {
            self.toast(
                Toast::warning(reason.to_string()).key("action-disabled:namespace::Create"),
                cx,
            );
            return;
        }
        let context = self
            .deps
            .sessions
            .get(&self.cluster)
            .map_or_else(|| self.cluster.to_string(), |s| s.context().to_string());
        let (cluster, creator, host) = (
            self.cluster.clone(),
            actions.namespace_creator(),
            workspace.downgrade(),
        );
        workspace.update(cx, |workspace, cx| {
            let dialog = cx
                .new(|cx| CreateNamespaceDialog::new(cluster, context, creator, host, window, cx));
            workspace.show_modal(dialog.clone(), window, cx);
            dialog.update(cx, |dialog, cx| dialog.focus_name(window, cx));
        });
    }
}
