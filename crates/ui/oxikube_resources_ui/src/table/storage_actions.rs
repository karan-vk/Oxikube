//! The storage actions of a table (E12-S15): "Resize…" on a claim and "Set as Default" on a
//! storage class open their dialog; `shift-r` and `shift-s` run the same actions from the
//! keyboard.
//!
//! Both are row actions of the kinds they apply to, so the context menu and the palette offer
//! them through the registry like any other, with the read-only reason when the cluster is
//! read-only. The key on any other kind says so in a toast.

use gpui::{Context, Window};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ResourceRef;

use super::actions::{ResizeClaim, SetDefaultClass};
use super::view::ResourceTable;
use crate::actions::ResourceActions;
use crate::kinds::storage::open;

impl ResourceTable {
    /// Opens the dialog of `command` (`pvc::Resize` or `storageclass::SetDefault`) for the first
    /// of `targets`: both act on one object, so a selection acts on the cursor row (and
    /// `run_action` already said so).
    pub(super) fn begin_storage_dialog(
        &mut self,
        actions: &ResourceActions,
        command: CommandId,
        targets: Vec<ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.as_ref().and_then(|ws| ws.upgrade()) else {
            tracing::warn!("a resource table without a workspace cannot open a storage dialog");
            return;
        };
        let Some(target) = targets.into_iter().next() else {
            self.say_no_row(cx);
            return;
        };
        let flows = actions.storage();
        if command == CommandId::PVC_RESIZE {
            open::open_resize(&workspace, flows, target, window, cx);
        } else {
            open::open_default_class(&workspace, flows, target, window, cx);
        }
    }

    /// `shift-r`: resize the cursor row's claim.
    pub(super) fn on_resize_claim(
        &mut self,
        _: &ResizeClaim,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::PVC_RESIZE, targets, window, cx);
    }

    /// `shift-s`: make the cursor row's storage class the default.
    pub(super) fn on_set_default_class(
        &mut self,
        _: &SetDefaultClass,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.action_targets(cx);
        self.run_action(CommandId::STORAGECLASS_SET_DEFAULT, targets, window, cx);
    }
}
