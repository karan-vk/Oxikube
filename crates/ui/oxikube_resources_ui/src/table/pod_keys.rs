//! The pod keys (E12-S03): `ctrl-k` force deletes, `z` sanitizes, `o` shows the node, `shift-j`
//! jumps to the owner.
//!
//! Each is a view action that stands for a command (`pod::ForceDelete`, `pod::Sanitize`,
//! `pod::ShowNode`, `pod::JumpToOwner`), so the key, the context menu, the palette and an agent
//! run one behaviour; the table never calls a port. The default bindings are in a `kind == Pod`
//! section of the per-OS keymap files, so the keys do nothing on other tables. Force delete, show
//! node and jump to owner act on the cursor row and say so when several rows are selected; a
//! sweep acts on the table's view, not on the selection.

use gpui::{Context, Window};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::PodPhase;
use oxikube_workspace::Toast;

use super::actions::{JumpToOwnerSelected, KillSelected, SanitizeSelected, ShowNodeSelected};
use super::view::ResourceTable;

impl ResourceTable {
    /// `ctrl-k`: the `pod::ForceDelete` row action, on the cursor row.
    pub(super) fn on_kill(
        &mut self,
        _: &KillSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_FORCE_DELETE, targets, window, cx);
    }

    /// `z`: the `pod::Sanitize` row action, over the table's view.
    pub(super) fn on_sanitize(
        &mut self,
        _: &SanitizeSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_SANITIZE, targets, window, cx);
    }

    /// `o`: the `pod::ShowNode` row action, on the cursor row.
    pub(super) fn on_show_node(
        &mut self,
        _: &ShowNodeSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_SHOW_NODE, targets, window, cx);
    }

    /// `shift-j`: the `pod::JumpToOwner` row action, on the cursor row.
    pub(super) fn on_jump_to_owner(
        &mut self,
        _: &JumpToOwnerSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::POD_JUMP_TO_OWNER, targets, window, cx);
    }

    /// The Completed (`Succeeded`) and `Failed` pods among the rows in view, in row order. This is
    /// the list `pod::Sanitize` is asked to sweep; the handler reads every pod again before it
    /// deletes it, so a row that went stale is skipped there.
    pub fn finished_pods_in_view(&self, cx: &gpui::App) -> Vec<ResourceRef> {
        self.read_rows(cx, |rows| {
            rows.rows()
                .iter()
                .filter_map(|object| {
                    let pod = object.resource()?;
                    // Only `status.phase` is read: this runs on the UI thread, once per row.
                    PodPhase::parse(pod.get_str("/status/phase"))
                        .is_terminal()
                        .then(|| {
                            ResourceRef::new(
                                self.cluster.clone(),
                                self.kind.gvk.clone(),
                                pod.meta.namespace.clone(),
                                pod.meta.name.clone(),
                            )
                        })
                })
                .collect()
        })
    }

    /// Sweeps the Completed and Failed pods of the view: dispatches one `pod::Sanitize` for them,
    /// which the bus confirms with the count. Says so instead when there are none.
    pub(super) fn sanitize_view(&mut self, cx: &mut Context<Self>) {
        let targets = self.finished_pods_in_view(cx);
        if targets.is_empty() {
            self.toast(
                Toast::info("No Completed or Failed pods in this view").key("action-sanitize-none"),
                cx,
            );
            return;
        }
        // One namespace, or none (every namespace is selected): what the audit record names.
        let namespace = targets
            .first()
            .and_then(|first| first.namespace().map(str::to_owned))
            .filter(|ns| targets.iter().all(|t| t.namespace() == Some(ns.as_str())));
        let command = Command::PodSanitize {
            cluster: self.cluster.clone(),
            namespace,
            targets,
        };
        self.deps.dispatcher.dispatch(command, cx);
    }
}
