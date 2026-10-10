//! [`ResourceViews`]: one per window; the kind view (E07-S11's [`KindViews`]) that opens
//! resource tables in the window's cluster tabs, and the UI-thread side of the table commands.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, Task, WeakEntity, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::kinds::{ResourceKind, Verb};
use oxikube_ports::FsPort;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{ClusterTabs, OpenOptions, Toast, Workspace};

use super::commands::ViewRequest;
use crate::detail::DetailTab;
use crate::navigate::{KindViews, OpenKind};
use crate::table::{ResourceTable, ResourceTableDeps, ResourceTableEvent, item_key};

/// What [`ResourceViews`] needs.
#[derive(Clone)]
pub struct ResourceViewsDeps {
    /// What every table is built over.
    pub table: ResourceTableDeps,
    /// The window's cluster tabs, where the tables open.
    pub tabs: WeakEntity<ClusterTabs>,
    /// Where "save YAML" writes (the file the user picked in the save dialog).
    pub fs: Arc<dyn FsPort>,
}

/// How long a [`ResourceViews::set_filter`] waits for its table to open. The jump bar sends the
/// list and its filter together, so a table that has not opened by then never will for this
/// jump (no discovery yet, no tab): a later, unrelated open must not take the filter.
pub(crate) const PENDING_FILTER_TTL: Duration = Duration::from_secs(10);

/// A filter that arrived before its table, and when.
struct PendingFilter {
    text: String,
    at: Instant,
}

/// A cluster's discovered kinds, with the discovery port they came from (a reconnect hands out
/// a new port, and the cache starts over).
pub(super) struct Kinds {
    pub(super) port: usize,
    pub(super) kinds: Arc<[ResourceKind]>,
}

/// Opens resource tables and runs the resource commands of one window. See the
/// [module docs](super).
pub struct ResourceViews {
    pub(super) deps: ResourceViewsDeps,
    pub(super) kinds: HashMap<ClusterId, Kinds>,
    /// The CRD navigation in flight (`crd::OpenResources`, E07-S07), and the reads of the
    /// versions a custom resource table's kind is served at.
    pub(super) crds: super::crd::CrdTasks,
    /// The sidebar navigation in flight (a newer click replaces, and so cancels, it).
    navigate_task: Option<Task<()>>,
    /// The `resource::OpenList` waiting on discovery (a newer one replaces, and so cancels, it).
    open_task: Option<Task<()>>,
    /// Filters (`table::SetFilter`) that arrived before their table was open: the jump bar sends
    /// the list and its filter together, and either may be applied first. The newest per kind
    /// wins; a table takes its filter when it opens, and one older than [`PENDING_FILTER_TTL`] is
    /// dropped unused.
    pending_filters: HashMap<(ClusterId, Gvk), PendingFilter>,
    /// When the open of a kind's list failed (no discovery yet, no tab): a filter that arrives
    /// right after it, from the same jump, has no table to wait for and is dropped.
    failed_opens: HashMap<(ClusterId, Gvk), Instant>,
    /// The "save YAML" in flight: the dialog, then the write (a newer one replaces it).
    pub(super) save_task: Option<Task<()>>,
    _requests: Task<()>,
}

impl ResourceViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`ResourceCommandSink`](super::ResourceCommandSink)) and registers it with [`KindViews`],
    /// so `resource::OpenList` for a cluster tab of this window opens the kind's table.
    pub fn start(
        deps: ResourceViewsDeps,
        mut requests: UnboundedReceiver<ViewRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let views = cx.new(|cx| {
            let pump = cx.spawn_in(window, async move |this, cx| {
                while let Some(request) = requests.next().await {
                    let applied = this.update_in(cx, |views: &mut Self, window, cx| {
                        views.apply_in(request, window, cx)
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            });
            Self {
                deps,
                kinds: HashMap::new(),
                crds: Default::default(),
                navigate_task: None,
                open_task: None,
                pending_filters: HashMap::new(),
                failed_opens: HashMap::new(),
                save_task: None,
                _requests: pump,
            }
        });
        let weak = views.downgrade();
        KindViews::register(cx, move |request, workspace, window, cx| {
            weak.upgrade().is_some_and(|views| {
                views.update(cx, |views, cx| {
                    views.open_kind(request, workspace, window, cx)
                })
            })
        });
        views
    }

    /// Applies one request, with the window it opens views in: what [`Self::apply`] does, plus
    /// opening the detail drawer for `Open`, pinning it for `PinDetail`, showing its YAML or
    /// Describe tab for `ViewYaml` and `ViewDescribe`, and focusing the filter bar for
    /// `FocusFilter`.
    pub fn apply_in(&mut self, request: ViewRequest, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            ViewRequest::Open(target) => {
                self.open_detail(&target, window, cx);
                self.apply(ViewRequest::Open(target), cx);
            }
            ViewRequest::PinDetail(target) => {
                self.pin_detail(&target, window, cx);
            }
            ViewRequest::ViewYaml(target) => {
                self.show_detail_tab(&target, DetailTab::Yaml, window, cx);
            }
            ViewRequest::ViewDescribe(target) => {
                self.show_detail_tab(&target, DetailTab::Describe, window, cx);
            }
            ViewRequest::Find { target, pattern } => {
                self.find_in_detail(&target, pattern.as_deref(), window, cx);
            }
            ViewRequest::FocusFilter { cluster, gvk } => {
                // The tab's active table (a kind has one table per cluster tab).
                if let Some(table) = self.tables(&cluster, &gvk, cx).into_iter().next() {
                    table.update(cx, |table, cx| table.focus_filter_on_command(window, cx));
                }
            }
            ViewRequest::SetFilter { cluster, gvk, text } => {
                self.set_filter(cluster, gvk, text, window, cx);
            }
            ViewRequest::OpenCrdResources { cluster, name } => {
                self.open_crd_resources(&cluster, name, window, cx);
            }
            other => self.apply(other, cx),
        }
    }

    /// Applies one request that needs no window: tells the tables, writes the clipboard.
    /// `Open` only reaches the tables here (their `OpenDetail` event); [`Self::apply_in`] also
    /// opens the drawer, and `PinDetail`, `ViewYaml`, `ViewDescribe` and `FocusFilter` need it, so
    /// they do nothing here.
    pub fn apply(&mut self, request: ViewRequest, cx: &mut Context<Self>) {
        match request {
            ViewRequest::PinDetail(_)
            | ViewRequest::ViewYaml(_)
            | ViewRequest::ViewDescribe(_)
            | ViewRequest::OpenCrdResources { .. } => {}
            ViewRequest::ToggleWide { cluster, gvk } => {
                for table in self.tables(&cluster, &gvk, cx) {
                    table.update(cx, |table, cx| table.toggle_wide(cx));
                }
            }
            ViewRequest::OpenCrdList(cluster) => {
                self.open_command(&cluster, &crate::crds::crd_gvk(), cx)
            }
            ViewRequest::CopyLabel {
                target,
                key,
                annotation,
            } => self.copy_label(&target, &key, annotation, cx),
            ViewRequest::Open(target) => {
                for table in self.tables(&target.cluster, &target.gvk, cx) {
                    let target = target.clone();
                    table.update(cx, |table, cx| table.open_detail(target, cx));
                }
            }
            ViewRequest::CopyYaml(target) => self.copy_yaml(&target, cx),
            ViewRequest::SaveYaml(target) => self.save_yaml(&target, cx),
            ViewRequest::ToggleManagedFields(target) => self.toggle_managed_fields(&target, cx),
            ViewRequest::RefreshDescribe(target) => self.refresh_describe(&target, cx),
            ViewRequest::NextMatch(target) => self.step_detail_match(&target, true, cx),
            ViewRequest::PreviousMatch(target) => self.step_detail_match(&target, false, cx),
            ViewRequest::CopyName(target) => {
                cx.write_to_clipboard(ClipboardItem::new_string(target.name.to_string()));
            }
            ViewRequest::RetryFeed { cluster, gvk } => {
                for table in self.tables(&cluster, &gvk, cx) {
                    table.update(cx, |table, cx| table.retry_feed(cx));
                }
            }
            ViewRequest::SelectAll { cluster, gvk } => {
                for table in self.tables(&cluster, &gvk, cx) {
                    table.update(cx, |table, cx| table.select_all(cx));
                }
            }
            // Need the window; see `apply_in`.
            ViewRequest::FocusFilter { .. }
            | ViewRequest::SetFilter { .. }
            | ViewRequest::Find { .. } => {}
        }
    }

    /// Opens the table `request` names when `workspace` is the tab of a connected cluster in
    /// this window: at once when discovery served the kind already, else once it resolves (a
    /// kind the cluster does not serve gets a notice in the tab). `false` when the tab is another
    /// window's or the cluster is not connected, so the next kind view (or the "no list view"
    /// notice) answers.
    pub fn open_kind(
        &mut self,
        request: &OpenKind,
        workspace: &Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let OpenKind { cluster, gvk } = request;
        let ours = self.deps.tabs.upgrade().is_some_and(|tabs| {
            tabs.read(cx)
                .tab(cluster)
                .is_some_and(|tab| tab.read(cx).workspace() == workspace)
        });
        if !ours {
            return false;
        }
        // A new open supersedes an earlier failure: only a failure of *this* open may make the
        // filter that follows it give up (a bare `:pods` that failed must not eat the next jump's).
        self.failed_opens.remove(&(cluster.clone(), gvk.clone()));
        let Some(discovery) = self
            .deps
            .table
            .sessions
            .get(cluster)
            .and_then(|s| s.discovery())
        else {
            self.open_failed(cluster, gvk, cx);
            return false;
        };
        let port = port_id(&discovery);
        let cached = self
            .kinds
            .get(cluster)
            .filter(|kinds| kinds.port == port)
            .and_then(|kinds| kinds.kinds.iter().find(|kind| kind.gvk == *gvk).cloned());
        if let Some(kind) = cached {
            self.open_list(cluster, kind, window, cx);
            return true;
        }
        let (cluster, gvk) = (cluster.clone(), gvk.clone());
        let lookup = gvk.clone();
        let resolve = spawn_kube(cx, async move { discovery.resolve(&lookup).await });
        let workspace = workspace.downgrade();
        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let kind = match resolve.await {
                Ok(Ok(kind)) => kind,
                Ok(Err(error)) => {
                    tracing::warn!(%error, %cluster, %gvk, "discovery failed: cannot open the list");
                    None
                }
                Err(error) => {
                    tracing::warn!(%error, "discovery task failed");
                    None
                }
            };
            this.update_in(cx, |views, window, cx| match kind {
                Some(kind) => {
                    views.open_list(&cluster, kind, window, cx);
                }
                None => {
                    // Nothing will open for a filter to wait for.
                    views.open_failed(&cluster, &gvk, cx);
                    let toast = Toast::info(format!("The cluster does not serve {}.", gvk.kind));
                    workspace
                        .update(cx, |ws, cx| {
                            ws.show_toast(toast, cx);
                        })
                        .ok();
                }
            })
            .ok();
        }));
        true
    }

    /// `table::SetFilter`: types `text` into the filter bar of the table of `gvk` in `cluster`, or
    /// keeps it for the table that is about to open.
    fn set_filter(
        &mut self,
        cluster: ClusterId,
        gvk: Gvk,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.tables(&cluster, &gvk, cx).into_iter().next() {
            Some(table) => {
                self.pending_filters.remove(&(cluster, gvk));
                table.update(cx, |table, cx| table.set_filter_text(&text, window, cx));
            }
            None => {
                let at = cx.background_executor().now();
                self.pending_filters.retain(|_, p| fresh(p, at));
                let failed = self
                    .failed_opens
                    .remove(&(cluster.clone(), gvk.clone()))
                    .is_some_and(|failed| within_ttl(failed, at));
                // The list of this jump could not open: nothing for the filter to wait for.
                if !failed {
                    self.pending_filters
                        .insert((cluster, gvk), PendingFilter { text, at });
                }
            }
        }
    }

    /// The open of `gvk`'s list in `cluster` failed: the filter that came with it (before or
    /// after) must not wait for an unrelated later table.
    fn open_failed(&mut self, cluster: &ClusterId, gvk: &Gvk, cx: &App) {
        let now = cx.background_executor().now();
        self.pending_filters.remove(&(cluster.clone(), gvk.clone()));
        self.failed_opens.retain(|_, at| within_ttl(*at, now));
        self.failed_opens
            .insert((cluster.clone(), gvk.clone()), now);
    }

    /// Shows `cluster`'s tab and, in it, the table of `kind` (the open one, else a new one).
    /// `None` when the cluster has no tab.
    pub fn open_list(
        &mut self,
        cluster: &ClusterId,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ResourceTable>> {
        let Some(tab) = self
            .deps
            .tabs
            .upgrade()
            .and_then(|tabs| tabs.read(cx).tab(cluster).cloned())
        else {
            self.open_failed(cluster, &kind.gvk, cx);
            return None;
        };
        if let Some(tabs) = self.deps.tabs.upgrade() {
            tabs.update(cx, |tabs, cx| tabs.activate(cluster, window, cx));
        }
        let workspace = tab.read(cx).workspace().clone();
        if let Some(open) = self.tables(cluster, &kind.gvk, cx).into_iter().next() {
            let key = item_key(&kind.gvk).into();
            if let Some(id) = workspace.read(cx).find_item_by_key(&key, cx) {
                workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            }
            return Some(open);
        }
        let deps = self.deps.table.clone();
        self.failed_opens
            .remove(&(cluster.clone(), kind.gvk.clone()));
        let pending = self
            .pending_filters
            .remove(&(cluster.clone(), kind.gvk.clone()))
            .filter(|p| fresh(p, cx.background_executor().now()))
            .map(|p| p.text);
        let cluster = cluster.clone();
        let table = cx.new(|cx| ResourceTable::new(cluster, kind, deps, window, cx));
        // The tab's workspace hosts the table's dialogs (delete) and toasts.
        table.update(cx, |table, _| table.set_workspace(workspace.downgrade()));
        // The API server's warnings reach the user as a toast in this cluster's window.
        let toasts = workspace.downgrade();
        cx.subscribe(&table, move |_, _, event: &ResourceTableEvent, cx| {
            if let ResourceTableEvent::ApiWarning(warning) = event {
                let toast = warning_toast(warning);
                toasts.update(cx, |ws, cx| ws.show_toast(toast, cx)).ok();
            }
        })
        .detach();
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(table.clone()), options, window, cx)
        });
        self.load_versions(&table, cx);
        if let Some(text) = pending {
            table.update(cx, |table, cx| table.set_filter_text(&text, window, cx));
        }
        Some(table)
    }

    /// The open tables of `gvk` in `cluster`'s tab.
    pub fn tables(&self, cluster: &ClusterId, gvk: &Gvk, cx: &App) -> Vec<Entity<ResourceTable>> {
        let Some(tabs) = self.deps.tabs.upgrade() else {
            return Vec::new();
        };
        let Some(tab) = tabs.read(cx).tab(cluster) else {
            return Vec::new();
        };
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
            .into_iter()
            .filter(|table| table.read(cx).gvk() == gvk)
            .collect()
    }

    /// The sidebar went to the list of `plural` in API group `group`: finds the kind through
    /// discovery (cached per connection; a miss discovers again, for a CRD installed since) and
    /// dispatches `resource::OpenList`.
    pub fn navigate(
        &mut self,
        cluster: &ClusterId,
        group: &str,
        plural: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.deps.table.sessions.get(cluster) else {
            return;
        };
        let Some(discovery) = session.discovery() else {
            return;
        };
        let port = port_id(&discovery);
        let cached = self
            .kinds
            .get(cluster)
            .filter(|kinds| kinds.port == port)
            .and_then(|kinds| find_kind(&kinds.kinds, group, plural));
        if let Some(kind) = cached {
            self.open_command(cluster, &kind.gvk, cx);
            return;
        }
        let (cluster, group, plural) = (cluster.clone(), group.to_owned(), plural.to_owned());
        let discover = spawn_kube(cx, async move { discovery.discover().await });
        self.navigate_task = Some(cx.spawn(async move |this, cx| {
            let kinds = match discover.await {
                Ok(Ok(kinds)) => kinds,
                Ok(Err(error)) => {
                    tracing::warn!(%error, %cluster, "discovery failed: cannot open {plural}");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "discovery task failed");
                    return;
                }
            };
            this.update(cx, |views, cx| {
                let kinds: Arc<[ResourceKind]> = kinds.into();
                let found = find_kind(&kinds, &group, &plural);
                views.kinds.insert(cluster.clone(), Kinds { port, kinds });
                match found {
                    Some(kind) => views.open_command(&cluster, &kind.gvk, cx),
                    None => tracing::warn!(%cluster, group, plural, "the cluster does not serve this kind"),
                }
            })
            .ok();
        }));
    }

    pub(super) fn open_command(&self, cluster: &ClusterId, gvk: &Gvk, cx: &mut App) {
        let command = Command::ResourceOpenList {
            cluster: cluster.clone(),
            gvk: gvk.clone(),
        };
        self.deps.table.dispatcher.dispatch(command, cx);
    }
}

/// The identity of a discovery port, to tell a reconnect's new port from the old one.
pub(super) fn port_id<T: ?Sized>(port: &Arc<T>) -> usize {
    Arc::as_ptr(port).cast::<()>() as usize
}

/// The listable kind `plural` of `group`, preferring the preferred version.
pub fn find_kind(kinds: &[ResourceKind], group: &str, plural: &str) -> Option<ResourceKind> {
    let matches = |kind: &&ResourceKind| {
        *kind.gvk.group == *group && kind.plural == plural && kind.supports(Verb::List)
    };
    kinds
        .iter()
        .filter(matches)
        .find(|kind| kind.preferred)
        .or_else(|| kinds.iter().find(matches))
        .cloned()
}

/// The toast for an API server warning: its text, keyed by it so a repeat replaces rather than
/// stacks. The text was redacted by the adapter.
fn warning_toast(warning: &oxikube_ports::ApiWarning) -> Toast {
    Toast::warning(warning.text.clone())
        .title("API server warning")
        .key(format!("api-warning/{}/{}", warning.code, warning.text))
}

/// Whether `pending` is still young enough to be applied at `now`.
fn fresh(pending: &PendingFilter, now: Instant) -> bool {
    within_ttl(pending.at, now)
}

fn within_ttl(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < PENDING_FILTER_TTL
}
