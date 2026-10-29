//! [`FilesView`]: a pod's container filesystem, as a workspace item of the cluster's tab.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | the view, its deps, opening it, the item glue |
//! | `load` | opening the pod, listing a directory off the UI thread, the per-path cache |
//! | `download`, `upload` | the save and file dialogs, the drop from the OS, the question before a replacement |
//! | `progress` | the progress toasts, the ticker that keeps them and the panel current, cancel |
//! | `render` | the toolbar, the path bar, the states, the status line |
//! | `panel` | the transfers panel |
//! | `delegate` | the table's data source and its row menu |
//!
//! # Cost
//!
//! The list is the app's virtualised `Table`: only the visible rows are drawn, and the cells of
//! every row are made once when a listing lands ([`FilesModel`]). A listing is read off the UI
//! thread (`spawn_kube`); the listing of each directory is cached for the life of the tab and
//! refreshed by hand. Transfers run on the Tokio bridge; the view reads the app's
//! `TransferBoard` at most every 100 ms, and only while something runs, and redraws through
//! `notify_coalesced`.

mod delegate;
mod download;
mod load;
mod panel;
mod progress;
mod render;
mod upload;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use oxikube_app::exec::ExecContainer;
use oxikube_app::files::{FileRunner, TransferId};
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::IconName;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_ui::table::{TableEvent, TableHandle, TableOptions};
use oxikube_workspace::{Item, ItemEvent, ItemHandle, OpenOptions, TabContent, Workspace};

pub use delegate::FilesDelegate;

use super::commands::FilesRequest;
use super::model::{FilesModel, PreparedListing};

/// What every file browser is built over.
#[derive(Clone)]
pub struct FilesViewDeps {
    /// Runs the listing, the downloads and the uploads (through the bus, so the guard applies).
    pub runner: FileRunner,
}

/// Where the view is in showing a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// Reading the pod to find its containers.
    Opening,
    /// Listing the directory.
    Loading,
    /// The listing is shown.
    Ready,
    /// The container has no shell tools: the clear empty state (not an error loop).
    NoShell(SharedString),
    /// The listing failed; the text says why.
    Failed(SharedString),
}

/// A pod's files: the container's directories, downloads, uploads and the transfers panel.
pub struct FilesView {
    pub(crate) pod: ResourceRef,
    pub(crate) deps: FilesViewDeps,
    pub(crate) workspace: Option<WeakEntity<Workspace>>,
    pub(crate) focus: FocusHandle,
    pub(crate) table: TableHandle<FilesDelegate>,
    pub(crate) path_input: Entity<InputState>,
    /// The path bar is the text field rather than the breadcrumb.
    pub(crate) editing_path: bool,
    pub(crate) containers: Vec<ExecContainer>,
    pub(crate) container: Option<Arc<str>>,
    pub(crate) path: String,
    pub(crate) state: LoadState,
    /// The listings read so far, per container and path; a refresh replaces one.
    pub(crate) cache: HashMap<(Arc<str>, String), PreparedListing>,
    /// Names the newest listing request; an older one that lands late is dropped.
    pub(crate) generation: u64,
    pub(crate) load_task: Option<Task<()>>,
    /// Transfers and prompts in flight; closing the tab drops them (and so stops the transfers).
    pub(crate) tasks: Vec<Task<()>>,
    /// The progress ticker runs.
    pub(crate) ticking: bool,
    /// The board's version at the last tick.
    pub(crate) seen_version: u64,
    /// The transfers whose progress toast is on screen.
    pub(crate) toasted: HashSet<TransferId>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for FilesView {}

impl Focusable for FilesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FilesView {
    /// A browser of `request.pod`'s files. It reads the pod and the first directory as soon as it
    /// is built.
    pub fn new(
        request: FilesRequest,
        deps: FilesViewDeps,
        workspace: Option<WeakEntity<Workspace>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = FilesDelegate::new(cx.entity().downgrade());
        let options = TableOptions {
            sortable: true,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: true,
        };
        let table = TableHandle::with_options(delegate, options, window, cx);
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("/path/in/container"));
        let mut this = Self {
            pod: request.pod.clone(),
            deps,
            workspace,
            focus: cx.focus_handle(),
            table,
            path_input,
            editing_path: false,
            containers: Vec::new(),
            container: None,
            path: request.path.clone().unwrap_or_default(),
            state: LoadState::Opening,
            cache: HashMap::new(),
            generation: 0,
            load_task: None,
            tasks: Vec::new(),
            ticking: false,
            seen_version: 0,
            toasted: HashSet::new(),
            _subscriptions: Vec::new(),
        };
        this.wire(window, cx);
        this.open_pod(request.container, request.path, cx);
        this
    }

    /// Shows the pod's files in `workspace`: the open browser of that pod (moved to the container
    /// and directory the request names), else a new tab. Returns the view.
    pub fn open(
        workspace: &Entity<Workspace>,
        request: FilesRequest,
        deps: FilesViewDeps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<FilesView> {
        let key = item_key(&request.pod);
        if let Some(open) = Self::find(workspace, &key, cx) {
            let id = open.entity_id();
            workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            open.update(cx, |view, cx| view.retarget(&request, cx));
            return open;
        }
        let weak = workspace.downgrade();
        let view = cx.new(|cx| FilesView::new(request, deps, Some(weak), window, cx));
        let item: Box<dyn ItemHandle> = Box::new(view.clone());
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
        view
    }

    /// The file browser of the pod with item key `key` open in `workspace`, if any.
    pub fn find(workspace: &Entity<Workspace>, key: &str, cx: &App) -> Option<Entity<FilesView>> {
        workspace
            .read(cx)
            .items_of_type::<FilesView>()
            .into_iter()
            .find(|view| view.read(cx).key() == key)
    }

    /// The pod the view shows.
    pub fn pod(&self) -> &ResourceRef {
        &self.pod
    }

    /// The directory shown (or being loaded).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The container shown.
    pub fn container(&self) -> Option<&str> {
        self.container.as_deref()
    }

    /// Where the view is.
    pub fn state(&self) -> &LoadState {
        &self.state
    }

    /// The rows, sort and selection.
    pub fn model<'a>(&self, cx: &'a App) -> &'a FilesModel {
        &self.table.delegate(cx).model
    }

    /// The tab's key: one browser per pod.
    pub fn key(&self) -> String {
        item_key(&self.pod)
    }

    fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let events = self.table.on_event(cx, move |event, cx| {
            weak.update(cx, |view, cx| view.on_table_event(event, cx))
                .ok();
        });
        // Clicking a header or a row focuses the library's table; the keys live on this view.
        let refocus = cx.on_focus(&self.table.focus_handle(cx), window, |view, window, cx| {
            window.focus(&view.focus, cx);
        });
        let input = cx.subscribe_in(
            &self.path_input.clone(),
            window,
            |view, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => view.commit_path(window, cx),
                InputEvent::Blur if view.editing_path => view.stop_editing_path(cx),
                _ => {}
            },
        );
        self._subscriptions.extend([events, refocus, input]);
    }

    fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        match event {
            TableEvent::SelectRow(row) => {
                self.table
                    .update_quiet(cx, |delegate| delegate.model.select(*row));
                cx.notify();
            }
            TableEvent::ActivateRow(row) => self.open_row(*row, cx),
            TableEvent::SortChanged { .. } => cx.notify(),
            _ => {}
        }
    }
}

/// The key of a pod's browser tab.
pub fn item_key(pod: &ResourceRef) -> String {
    format!("files/{}/{}", pod.namespace().unwrap_or("-"), pod.name)
}

impl Item for FilesView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(format!("Files: {}", self.pod.name)).icon(IconName::FolderOpen)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(self.key().into())
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Stop listing now rather than when the entity is dropped. A transfer started here stops
        // with the tab: its task is dropped, which aborts the exec session.
        self.load_task = None;
        self.tasks.clear();
    }
}
