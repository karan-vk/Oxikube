//! Opening the pod, listing a directory, and moving around.

use std::sync::Arc;

use gpui::{Context, Window};
use oxikube_app::files::{FileEntry, OpenedPod, join, parent_of};
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::{FilesView, LoadState};
use crate::files::commands::FilesRequest;
use crate::files::model::PreparedListing;

/// A path as the view keeps it: absolute paths lose a trailing slash, an empty one is the root.
pub(crate) fn normalise(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "/".to_owned();
    }
    let stripped = trimmed.trim_end_matches('/');
    if stripped.is_empty() {
        "/".to_owned()
    } else {
        stripped.to_owned()
    }
}

impl FilesView {
    /// Reads the pod for its containers, then lists the first directory.
    pub(crate) fn open_pod(
        &mut self,
        container: Option<String>,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.state = LoadState::Opening;
        let service = self.deps.runner.service().clone();
        let pod = self.pod.clone();
        let work = spawn_kube(
            cx,
            async move { service.open(&pod, container.as_deref()).await },
        );
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let opened = work.await.unwrap_or_else(|error| Err(error.into()));
            this.update(cx, |view, cx| view.opened(opened, path, cx))
                .ok();
        }));
        cx.notify();
    }

    fn opened(
        &mut self,
        opened: OxiResult<OpenedPod>,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match opened {
            Ok(opened) => {
                self.containers = opened.containers.containers().to_vec();
                self.container = Some(opened.container.clone());
                let path = path
                    .or_else(|| {
                        self.deps
                            .runner
                            .service()
                            .last_path(&self.pod, &opened.container)
                    })
                    .unwrap_or_else(|| "/".to_owned());
                self.load(normalise(&path), false, cx);
            }
            Err(error) => {
                self.state = LoadState::Failed(error.message().to_owned().into());
                cx.notify();
            }
        }
    }

    /// Moves an open browser to what a new `pod::BrowseFiles` asks for.
    pub(crate) fn retarget(&mut self, request: &FilesRequest, cx: &mut Context<Self>) {
        if let Some(container) = &request.container
            && self.container.as_deref() != Some(container.as_str())
        {
            self.select_container(container, cx);
        }
        if let Some(path) = &request.path {
            self.navigate(path, cx);
        }
    }

    /// Lists `path` in the current container: from the cache unless `force`.
    pub(crate) fn load(&mut self, path: String, force: bool, cx: &mut Context<Self>) {
        let Some(container) = self.container.clone() else {
            return;
        };
        self.path = path.clone();
        self.editing_path = false;
        self.generation += 1;
        let generation = self.generation;
        if !force
            && let Some(prepared) = self.cache.get(&(container.clone(), path.clone())).cloned()
        {
            self.show(&path, &prepared, cx);
            return;
        }
        self.state = LoadState::Loading;
        let service = self.deps.runner.service().clone();
        let pod = self.pod.clone();
        let (sort, show_hidden) = {
            let model = self.model(cx);
            (model.sort(), model.show_hidden())
        };
        // The cells of 10 000 rows are made where the listing is read, not on the UI thread.
        let work = spawn_kube(cx, async move {
            let listing = service.list(&pod, Some(&container), &path).await?;
            Ok((
                listing.path.clone(),
                PreparedListing::new(&listing, sort, show_hidden),
            ))
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let listing = work.await.unwrap_or_else(|error| Err(error.into()));
            this.update(cx, |view, cx| view.loaded(generation, listing, cx))
                .ok();
        }));
        cx.notify();
    }

    fn loaded(
        &mut self,
        generation: u64,
        listing: OxiResult<(String, PreparedListing)>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        match listing {
            Ok((path, prepared)) => {
                if let Some(container) = &self.container {
                    self.cache
                        .insert((container.clone(), path.clone()), prepared.clone());
                }
                self.show(&path, &prepared, cx);
            }
            Err(error) => {
                self.table.update(cx, |delegate| delegate.model.clear());
                self.state = if error.kind() == ErrorKind::Unsupported {
                    LoadState::NoShell(error.message().to_owned().into())
                } else {
                    LoadState::Failed(error.message().to_owned().into())
                };
                cx.notify();
            }
        }
    }

    fn show(&mut self, path: &str, prepared: &PreparedListing, cx: &mut Context<Self>) {
        if let Some(container) = &self.container {
            self.deps
                .runner
                .service()
                .remember_path(&self.pod, container, path);
        }
        self.table
            .update(cx, |delegate| delegate.model.install(prepared));
        self.state = LoadState::Ready;
        cx.notify();
    }

    /// Goes to `path` (absolute, or as typed in the path bar).
    pub fn navigate(&mut self, path: &str, cx: &mut Context<Self>) {
        self.load(normalise(path), false, cx);
    }

    /// Goes to the directory above.
    pub fn go_up(&mut self, cx: &mut Context<Self>) {
        let parent = parent_of(&self.path);
        self.navigate(&parent, cx);
    }

    /// Lists the directory again, ignoring the cached listing.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(self.path.clone(), true, cx);
    }

    /// Shows or hides dot files.
    pub fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        let show = !self.table.delegate(cx).model.show_hidden();
        self.table
            .update(cx, |delegate| delegate.model.set_show_hidden(show));
        cx.notify();
    }

    /// Switches to another container of the pod, at the directory last used there.
    pub fn select_container(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.container.as_deref() == Some(name)
            || !self.containers.iter().any(|c| &*c.name == name)
        {
            return;
        }
        let name: Arc<str> = Arc::from(name);
        let path = self
            .deps
            .runner
            .service()
            .last_path(&self.pod, &name)
            .unwrap_or_else(|| "/".to_owned());
        self.container = Some(name);
        self.table.update(cx, |delegate| delegate.model.clear());
        self.load(normalise(&path), false, cx);
    }

    /// Opens the directory (or link) shown at `row`.
    pub(crate) fn open_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let entry = self
            .table
            .delegate(cx)
            .model
            .row(row)
            .map(|row| row.entry.clone());
        if let Some(entry) = entry {
            self.open_entry(&entry, cx);
        }
    }

    /// Opens `entry` when it is a directory or a link; a file does nothing.
    pub fn open_entry(&mut self, entry: &FileEntry, cx: &mut Context<Self>) {
        if entry.is_navigable() {
            let target = join(&self.path, &entry.name);
            self.navigate(&target, cx);
        }
    }

    /// Copies the entry's path in the container to the clipboard.
    pub(crate) fn copy_path(&mut self, entry: &FileEntry, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(join(
            &self.path,
            &entry.name,
        )));
        self.toast(Toast::info("Path copied"), cx);
    }

    /// The selected entry opened or downloaded by the toolbar.
    pub(crate) fn selected_entry(&self, cx: &gpui::App) -> Option<FileEntry> {
        self.table.delegate(cx).model.selected().cloned()
    }

    /// Opens the path bar's text field on the current path.
    pub fn start_editing_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_path = true;
        let path = self.path.clone();
        self.path_input.update(cx, |input, cx| {
            input.set_value(path, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn stop_editing_path(&mut self, cx: &mut Context<Self>) {
        self.editing_path = false;
        cx.notify();
    }

    /// Goes to the path typed in the path bar.
    pub(crate) fn commit_path(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let typed = self.path_input.read(cx).value().to_string();
        self.navigate(&typed, cx);
    }
}
