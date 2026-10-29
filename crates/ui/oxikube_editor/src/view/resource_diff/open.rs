//! `resource::DiffWith` on the window's [`EditorViews`]: the picker, and the diff tab.

use gpui::{AppContext as _, Context, Window};
use oxikube_app::diff::DiffRequest;
use oxikube_domain::ids::ResourceRef;

use super::data::DiffState;
use super::picker::DiffTargetDialog;
use super::view::ResourceDiffView;
use crate::view::controller::EditorViews;

impl EditorViews {
    /// `resource::DiffWith` without a second object: the "Diff with..." picker in the object's
    /// cluster tab (the window's own workspace without one).
    pub(in crate::view) fn pick_diff_target(
        &mut self,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.deps.host.clone();
        let workspace = host
            .workspace(&target.cluster, cx)
            .or_else(|| self.deps.window_workspace.upgrade());
        let Some(workspace) = workspace else {
            return;
        };
        host.show(&target.cluster, window, cx);
        let (catalog, dispatcher) = (host.catalog(), self.deps.dispatcher.clone());
        let own_label = host.cluster_label(&target.cluster);
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| {
                DiffTargetDialog::new(target, own_label, catalog, dispatcher, window, cx)
            });
            workspace.show_modal(dialog, window, cx);
        });
    }

    /// The diff tab of `request` in `state`: updated when the comparison is open already, else
    /// opened (in the left object's cluster tab) and shown.
    pub(in crate::view) fn show_resource_diff(
        &mut self,
        request: DiffRequest,
        state: DiffState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.deps.host.clone();
        let cluster = request.left.cluster.clone();
        let workspace = host
            .workspace(&cluster, cx)
            .or_else(|| self.deps.window_workspace.upgrade());
        let Some(workspace) = workspace else {
            tracing::debug!("resource::DiffWith: no tab to show the diff in");
            return;
        };
        let key = ResourceDiffView::key_of(&request);
        let held = workspace
            .read(cx)
            .items_of_type::<ResourceDiffView>()
            .into_iter()
            .find(|view| ResourceDiffView::key_of(view.read(cx).request()) == key);
        if let Some(view) = held {
            view.update(cx, |view, cx| view.update_state(request, state, cx));
            return;
        }
        host.show(&cluster, window, cx);
        let dispatcher = self.deps.dispatcher.clone();
        let view = cx.new(|cx| ResourceDiffView::new(request.clone(), dispatcher, cx));
        view.update(cx, |view, cx| view.update_state(request, state, cx));
        workspace.update(cx, |workspace, cx| workspace.open_item(view, window, cx));
    }
}
