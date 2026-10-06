//! The detail drawer's side of [`ResourceViews`]: opening it for `resource::Open`, pinning it
//! as a tab for `resource::PinDetail`, and `resource::CopyLabel`.
//!
//! The drawer is a panel of the cluster tab's own workspace, added the first time a detail is
//! opened there; the detail it shows is one entity that is also a workspace item, so pinning
//! hands that entity to the workspace.

use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, Window};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_workspace::{OpenOptions, Workspace};

use super::controller::ResourceViews;
use crate::detail::{DetailDeps, DetailDrawer, DetailView, item_key};

impl ResourceViews {
    /// The workspace of `cluster`'s tab.
    pub(super) fn tab_workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>> {
        let tabs = self.deps.tabs.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?;
        Some(tab.read(cx).workspace().clone())
    }

    /// Shows the detail of `target` in `target.cluster`'s tab: the tab it is pinned as when it
    /// is, else the drawer (created and added to the right dock on first use). `None` when the
    /// cluster has no tab.
    pub fn open_detail(
        &mut self,
        target: &ResourceRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<DetailView>> {
        let workspace = self.tab_workspace(&target.cluster, cx)?;
        if let Some(tabs) = self.deps.tabs.upgrade() {
            tabs.update(cx, |tabs, cx| tabs.activate(&target.cluster, window, cx));
        }
        let key = item_key(target).into();
        if let Some(id) = workspace.read(cx).find_item_by_key(&key, cx) {
            workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            return self.detail_view(target, cx);
        }
        let drawer = match workspace.read(cx).panel::<DetailDrawer>() {
            Some(drawer) => drawer,
            None => {
                let drawer = cx.new(DetailDrawer::new);
                workspace.update(cx, |ws, cx| ws.add_panel(drawer.clone(), window, cx));
                drawer
            }
        };
        let deps = DetailDeps::from(&self.deps.table);
        let target = target.clone();
        Some(drawer.update(cx, |drawer, cx| drawer.show(target, &deps, cx)))
    }

    /// Pins the drawer's detail of `target` as a tab: the same entity moves from the drawer to
    /// the workspace's pane, state intact, and the drawer closes. Does nothing when the drawer
    /// is not showing `target` (or it is a tab already).
    pub fn pin_detail(
        &mut self,
        target: &ResourceRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<DetailView>> {
        let workspace = self.tab_workspace(&target.cluster, cx)?;
        let drawer = workspace.read(cx).panel::<DetailDrawer>()?;
        let view = drawer.update(cx, |drawer, cx| drawer.take(target, cx))?;
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(view.clone()), OpenOptions::default(), window, cx)
        });
        Some(view)
    }

    /// The open detail of `target`: its tab, else the drawer's.
    pub fn detail_view(&self, target: &ResourceRef, cx: &App) -> Option<Entity<DetailView>> {
        let workspace = self.tab_workspace(&target.cluster, cx)?;
        let pinned = workspace
            .read(cx)
            .items_of_type::<DetailView>()
            .into_iter()
            .find(|view| view.read(cx).target() == target);
        pinned.or_else(|| {
            let drawer = workspace.read(cx).panel::<DetailDrawer>()?;
            let view = drawer.read(cx).view()?.clone();
            (view.read(cx).target() == target).then_some(view)
        })
    }

    /// Puts `key=value` of the label (or annotation) on the clipboard.
    pub(super) fn copy_label(
        &self,
        target: &ResourceRef,
        key: &str,
        annotation: bool,
        cx: &mut Context<Self>,
    ) {
        let text = self
            .detail_view(target, cx)
            .and_then(|view| view.read(cx).copy_text(key, annotation));
        match text {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => tracing::debug!(%target, key, "no such label to copy"),
        }
    }
}
