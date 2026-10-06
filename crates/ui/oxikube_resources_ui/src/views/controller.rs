//! [`ResourceViews`]: one per window; the kind view (E07-S11's [`KindViews`]) that opens
//! resource tables in the window's cluster tabs, and the UI-thread side of the table commands.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, ClipboardItem, Context, Entity, Task, WeakEntity, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::kinds::{ResourceKind, Verb};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{ClusterTabs, OpenOptions, Toast, Workspace};

use super::commands::ViewRequest;
use crate::navigate::{KindViews, OpenKind};
use crate::table::{ResourceTable, ResourceTableDeps, ResourceTableEvent, item_key};

/// What [`ResourceViews`] needs.
#[derive(Clone)]
pub struct ResourceViewsDeps {
    /// What every table is built over.
    pub table: ResourceTableDeps,
    /// The window's cluster tabs, where the tables open.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// A cluster's discovered kinds, with the discovery port they came from (a reconnect hands out
/// a new port, and the cache starts over).
struct Kinds {
    port: usize,
    kinds: Arc<[ResourceKind]>,
}

/// Opens resource tables and runs the resource commands of one window. See the
/// [module docs](super).
pub struct ResourceViews {
    deps: ResourceViewsDeps,
    kinds: HashMap<ClusterId, Kinds>,
    /// The sidebar navigation in flight (a newer click replaces, and so cancels, it).
    navigate_task: Option<Task<()>>,
    /// The `resource::OpenList` waiting on discovery (a newer one replaces, and so cancels, it).
    open_task: Option<Task<()>>,
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
                    let applied = this.update(cx, |views: &mut Self, cx| views.apply(request, cx));
                    if applied.is_err() {
                        break;
                    }
                }
            });
            Self {
                deps,
                kinds: HashMap::new(),
                navigate_task: None,
                open_task: None,
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

    /// Applies one request.
    pub fn apply(&mut self, request: ViewRequest, cx: &mut Context<Self>) {
        match request {
            ViewRequest::Open(target) => {
                for table in self.tables(&target.cluster, &target.gvk, cx) {
                    let target = target.clone();
                    table.update(cx, |table, cx| table.open_detail(target, cx));
                }
            }
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
        let Some(discovery) = self
            .deps
            .table
            .sessions
            .get(cluster)
            .and_then(|s| s.discovery())
        else {
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

    /// Shows `cluster`'s tab and, in it, the table of `kind` (the open one, else a new one).
    /// `None` when the cluster has no tab.
    pub fn open_list(
        &mut self,
        cluster: &ClusterId,
        kind: ResourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ResourceTable>> {
        let tabs = self.deps.tabs.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?.clone();
        tabs.update(cx, |tabs, cx| tabs.activate(cluster, window, cx));
        let workspace = tab.read(cx).workspace().clone();
        if let Some(open) = self.tables(cluster, &kind.gvk, cx).into_iter().next() {
            let key = item_key(&kind.gvk).into();
            if let Some(id) = workspace.read(cx).find_item_by_key(&key, cx) {
                workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            }
            return Some(open);
        }
        let deps = self.deps.table.clone();
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

    fn open_command(&self, cluster: &ClusterId, gvk: &Gvk, cx: &mut App) {
        let command = Command::ResourceOpenList {
            cluster: cluster.clone(),
            gvk: gvk.clone(),
        };
        self.deps.table.dispatcher.dispatch(command, cx);
    }
}

/// The identity of a discovery port, to tell a reconnect's new port from the old one.
fn port_id<T: ?Sized>(port: &Arc<T>) -> usize {
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
