//! [`TerminalViews`]: one per window; applies the `terminal::*` requests (new, split, close) on
//! the UI thread, in the workspace that is shown.
//!
//! - **New** opens a local shell for a cluster in the bottom dock of its tab (adding the
//!   [`TerminalPanel`](super::TerminalPanel) when the tab has none), with the cluster's selected
//!   namespace; with no cluster tab shown, a plain shell tab in the window's own workspace.
//! - **Open** (E08-S08) opens a terminal running a given descriptor the same way (the log viewer's
//!   `kubectl logs -f`), in the dock of the descriptor's own cluster, with exactly that descriptor.
//! - **Split** opens a terminal running what the focused one runs, in the directory its shell is
//!   in now (the shown cluster's shell otherwise), in a new pane right of the focused terminal's
//!   pane, else of the active pane.
//! - **Close** closes the focused terminal, else the active pane's item when it is a terminal.
//! - **Reconnect** and **Restart** (E09-S12) start a new session in the same terminal (a pod's, a
//!   local shell's): the focused one, else the active pane's.
//!
//! Each request waits one turn first, so a palette that just closed has handed the focus back to
//! the terminal it was opened over.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, AppContext as _, Context, Entity, Task, WeakEntity, Window};
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{DockPosition, SplitDirection, Workspace};

use super::TerminalView;
use super::commands::TerminalRequest;
use super::descriptor::BackendDescriptor;
use super::host::TerminalHost;
use super::panel::ensure_terminal_panel;
use super::services::TerminalServices;

/// What [`TerminalViews`] needs.
#[derive(Clone)]
pub struct TerminalViewsDeps {
    /// Where the clusters' terminals go (the window's cluster tabs).
    pub host: Rc<dyn TerminalHost>,
    /// The window's own workspace: plain shells open here when no cluster tab is shown.
    pub window_workspace: WeakEntity<Workspace>,
    /// What every terminal is built over.
    pub services: TerminalServices,
}

/// Opens, splits and closes the terminals of one window. See the [module docs](self).
pub struct TerminalViews {
    deps: TerminalViewsDeps,
    _requests: Task<()>,
}

impl TerminalViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`TerminalViewSink`](super::TerminalViewSink)).
    pub fn start(
        deps: TerminalViewsDeps,
        mut requests: UnboundedReceiver<TerminalRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let pump = cx.spawn_in(window, async move |this, cx| {
                while let Some(request) = requests.next().await {
                    let applied = this.update_in(cx, |views: &mut Self, window, cx| {
                        views.apply(request, window, cx);
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            });
            Self {
                deps,
                _requests: pump,
            }
        })
    }

    /// Applies `request` on the next turn (see the [module docs](self)).
    pub fn apply(&mut self, request: TerminalRequest, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, move |this, window, cx| match request {
            TerminalRequest::New { cluster } => this.open_new(cluster, window, cx),
            TerminalRequest::Open { descriptor } => {
                this.open_in(descriptor.cluster().cloned(), descriptor, window, cx);
            }
            TerminalRequest::Split => this.split(window, cx),
            TerminalRequest::Close => this.close_focused(window, cx),
            TerminalRequest::Reconnect => this.recover(true, window, cx),
            TerminalRequest::Restart => this.recover(false, window, cx),
        });
    }

    /// Opens a new local shell for `cluster` (the shown cluster when `None`): see the
    /// [module docs](self). Does nothing when the cluster has no tab in this window.
    fn open_new(
        &mut self,
        cluster: Option<ClusterId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cluster = cluster.or_else(|| self.deps.host.active_cluster(cx));
        let descriptor = match &cluster {
            Some(cluster) => {
                let namespace = self.deps.host.namespace(cluster, cx);
                BackendDescriptor::local(Some(cluster.clone())).in_namespace(namespace)
            }
            None => BackendDescriptor::local(None),
        };
        self.open_in(cluster, descriptor, window, cx);
    }

    /// Opens a terminal running `descriptor` where its cluster's terminals go (see
    /// [`open_new`](Self::open_new)); a descriptor without a cluster opens in the window's own
    /// workspace, never in the shown cluster's.
    fn open_in(
        &mut self,
        cluster: Option<ClusterId>,
        descriptor: BackendDescriptor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cluster) = cluster else {
            if let Some(workspace) = self.deps.window_workspace.upgrade() {
                let view = self.build(descriptor, cx);
                workspace.update(cx, |ws, cx| ws.open_item(view, window, cx));
            }
            return;
        };
        let Some(workspace) = self.deps.host.workspace(&cluster, cx) else {
            return;
        };
        self.deps.host.show(&cluster, window, cx);
        let dispatcher = self.deps.services.dispatcher().cloned();
        ensure_terminal_panel(&workspace, Some(cluster), dispatcher, window, cx);
        let view = self.build(descriptor, cx);
        workspace.update(cx, |ws, cx| {
            let docked = ws.open_item_in_dock(
                Box::new(view.clone()),
                DockPosition::Bottom,
                true,
                window,
                cx,
            );
            if docked.is_none() {
                ws.open_item(view, window, cx);
            }
        });
    }

    /// Opens a terminal in a new pane: see the [module docs](self).
    fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((cluster, workspace)) = self.shown_workspace(cx) else {
            return;
        };
        let focused = focused_terminal(&workspace, window, cx);
        let descriptor = match &focused {
            Some(view) => view.read(cx).live_descriptor(cx),
            None => {
                let namespace = cluster
                    .as_ref()
                    .and_then(|cluster| self.deps.host.namespace(cluster, cx));
                BackendDescriptor::local(cluster).in_namespace(namespace)
            }
        };
        let view = self.build(descriptor, cx);
        workspace.update(cx, |ws, cx| {
            let pane = focused
                .as_ref()
                .and_then(|focused| {
                    ws.pane_group(cx)
                        .pane_for_item(focused.entity_id())
                        .cloned()
                })
                .map(|pane| pane.id());
            ws.open_item_in_split(Box::new(view), pane, SplitDirection::Right, window, cx);
        });
    }

    /// Closes the focused terminal, else the active pane's terminal.
    fn close_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, workspace)) = self.shown_workspace(cx) else {
            return;
        };
        if let Some(target) = target_terminal(&workspace, window, cx) {
            let id = target.entity_id();
            workspace.update(cx, |ws, cx| ws.close_item(id, window, cx));
        }
    }

    /// Starts a new session in the focused (else the active pane's) terminal: a pod's when
    /// `reconnect`, a local shell's otherwise. Does nothing for a terminal that is not in the
    /// matching kind and state (see [`TerminalView::reconnect`]).
    fn recover(&mut self, reconnect: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, workspace)) = self.shown_workspace(cx) else {
            return;
        };
        let Some(target) = target_terminal(&workspace, window, cx) else {
            return;
        };
        target.update(cx, |view, cx| {
            if reconnect {
                view.reconnect(cx);
            } else {
                view.restart(cx);
            }
        });
    }

    /// The workspace on screen: the shown cluster's, else the window's.
    fn shown_workspace(&self, cx: &App) -> Option<(Option<ClusterId>, Entity<Workspace>)> {
        if let Some(cluster) = self.deps.host.active_cluster(cx)
            && let Some(workspace) = self.deps.host.workspace(&cluster, cx)
        {
            return Some((Some(cluster), workspace));
        }
        Some((None, self.deps.window_workspace.upgrade()?))
    }

    fn build(&self, descriptor: BackendDescriptor, cx: &mut Context<Self>) -> Entity<TerminalView> {
        let services = self.deps.services.clone();
        cx.new(|cx| TerminalView::new(descriptor, services, cx))
    }
}

/// The terminal a command acts on: the focused one, else the active pane's when it is a terminal.
fn target_terminal(
    workspace: &Entity<Workspace>,
    window: &Window,
    cx: &App,
) -> Option<Entity<TerminalView>> {
    focused_terminal(workspace, window, cx).or_else(|| {
        let item = workspace.read(cx).active_item(cx)?;
        item.downcast::<TerminalView>()
    })
}

/// The terminal of `workspace` that has the focus.
fn focused_terminal(
    workspace: &Entity<Workspace>,
    window: &Window,
    cx: &App,
) -> Option<Entity<TerminalView>> {
    use gpui::Focusable as _;
    workspace
        .read(cx)
        .items_of_type::<TerminalView>()
        .into_iter()
        .find(|view| view.read(cx).focus_handle(cx).contains_focused(window, cx))
}
