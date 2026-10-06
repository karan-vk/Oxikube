//! The detail drawer's side of [`ResourceViews`]: opening it for `resource::Open`, pinning it
//! as a tab for `resource::PinDetail`, `resource::CopyLabel`, and the YAML tab's `resource::CopyYaml`
//! and `resource::SaveYaml` (E07-S06).
//!
//! The drawer is a panel of the cluster tab's own workspace, added the first time a detail is
//! opened there; the detail it shows is one entity that is also a workspace item, so pinning
//! hands that entity to the workspace.

use std::path::PathBuf;

use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, Window};
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{OpenOptions, Toast, Workspace};

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

    /// Shows or hides `managedFields` in the YAML tab of `target`'s open detail.
    pub(super) fn toggle_managed_fields(&self, target: &ResourceRef, cx: &mut Context<Self>) {
        if let Some(view) = self.detail_view(target, cx) {
            view.update(cx, |view, cx| view.toggle_managed_fields(cx));
        }
    }

    /// Reads the describe text of `target`'s open detail again.
    pub(super) fn refresh_describe(&self, target: &ResourceRef, cx: &mut Context<Self>) {
        if let Some(view) = self.detail_view(target, cx) {
            view.update(cx, |view, cx| view.refresh_describe(cx));
        }
    }

    /// Puts the YAML the detail of `target` shows on the clipboard: exactly the displayed text,
    /// so a Secret is copied masked.
    pub(super) fn copy_yaml(&self, target: &ResourceRef, cx: &mut Context<Self>) {
        let text = self
            .detail_view(target, cx)
            .and_then(|view| view.read(cx).yaml().map(str::to_owned));
        match text {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            None => {
                tracing::debug!(%target, "no YAML to copy");
                self.toast(target, Toast::info("The YAML is not ready yet."), cx);
            }
        }
    }

    /// Asks where to save the YAML the detail of `target` shows, then writes exactly that text
    /// through the `FsPort` (the file dialog is the platform's; nothing is written when it is
    /// cancelled). A Secret's file holds the masked text.
    pub(super) fn save_yaml(&mut self, target: &ResourceRef, cx: &mut Context<Self>) {
        let Some((text, name)) = self.detail_view(target, cx).and_then(|view| {
            let view = view.read(cx);
            view.yaml()
                .map(|text| (text.to_owned(), view.yaml_file_name()))
        }) else {
            tracing::debug!(%target, "no YAML to save");
            self.toast(target, Toast::info("The YAML is not ready yet."), cx);
            return;
        };
        let answer = cx.prompt_for_new_path(&save_directory(), Some(&name));
        let fs = self.deps.fs.clone();
        let target = target.clone();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            // Cancelled, or the platform could not show the dialog: nothing to write.
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            let written = this.update(cx, |_, cx| {
                let path = path.clone();
                spawn_kube(cx, async move { fs.write(&path, text.as_bytes()).await })
            });
            let Ok(written) = written else {
                return;
            };
            let result = match written.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |views, cx| {
                let toast = match result {
                    Ok(()) => Toast::success(format!("Saved {}", path.display())),
                    Err(error) => {
                        tracing::warn!(%error, "saving the YAML failed");
                        Toast::error(format!("Could not save the YAML: {}", error.message()))
                    }
                };
                views.toast(&target, toast, cx);
            })
            .ok();
        }));
    }

    /// Shows `toast` in `target`'s cluster tab.
    fn toast(&self, target: &ResourceRef, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = self.tab_workspace(&target.cluster, cx) {
            workspace.update(cx, |ws, cx| {
                ws.show_toast(toast, cx);
            });
        }
    }
}

/// Where the save dialog opens: the user's home directory, else the current one.
fn save_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
