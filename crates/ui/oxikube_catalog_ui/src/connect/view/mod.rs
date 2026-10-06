//! [`ConnectView`]: the body of a cluster tab whose session is not connected.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the entity, its session subscription, what its buttons do |
//! | `render.rs` | the body of each state |
//! | `parts.rs` | the pieces they share: the card, buttons, the selectable details |

mod parts;
mod render;

use futures::StreamExt as _;
use gpui::{ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Render, Task, Window};
use oxikube_app::{ClusterSession, SessionChange, SessionUpdate};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::ClusterSessionState;
use oxikube_workspace::ItemEvent;

use super::deps::ConnectDeps;
use super::model::{ConnectInfo, ConnectViewModel};

/// The connect view of one cluster. See the [module docs](super).
///
/// It follows its session through the manager's update stream (redrawing at most once per
/// frame), draws the body [`ConnectViewModel`] says, and turns its buttons into commands:
/// Retry is `cluster::Reconnect`, Cancel is `cluster::CancelConnect`, Connect is
/// `cluster::Connect`. It never calls a session or a port itself.
pub struct ConnectView {
    pub(super) deps: ConnectDeps,
    pub(super) cluster: ClusterId,
    pub(super) state: ClusterSessionState,
    pub(super) info: ConnectInfo,
    pub(super) model: ConnectViewModel,
    /// Whether the error details are expanded.
    pub(super) details_open: bool,
    pub(super) focus: FocusHandle,
    /// Applies session updates. Lives as long as the view.
    _watch_session: Task<()>,
}

impl EventEmitter<ItemEvent> for ConnectView {}

impl Focusable for ConnectView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConnectView {
    /// A view of `cluster`'s session, which must be open (the tab exists because it is).
    pub fn new(deps: ConnectDeps, cluster: ClusterId, cx: &mut Context<Self>) -> Self {
        // Subscribe before reading the session, so no update falls between the two.
        let mut updates = deps.sessions.subscribe();
        let watch_session = cx.spawn(async move |this, cx| {
            while let Some(item) = updates.next().await {
                let alive = this.update(cx, |this, cx| match item {
                    Ok(update) => this.apply_update(&update, cx),
                    // Missed some: read the session again instead of replaying.
                    Err(_) => this.sync(cx),
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            deps,
            cluster,
            state: ClusterSessionState::Disconnected,
            info: ConnectInfo {
                title: String::new(),
                context: String::new(),
                server: None,
                exec: Default::default(),
                terminal: false,
                sources: false,
            },
            model: ConnectViewModel::Content,
            details_open: false,
            focus: cx.focus_handle(),
            _watch_session: watch_session,
        };
        view.sync(cx);
        view
    }

    /// The cluster this view shows.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// What the view shows now.
    pub fn model(&self) -> &ConnectViewModel {
        &self.model
    }

    /// The session state the view shows.
    pub fn state(&self) -> &ClusterSessionState {
        &self.state
    }

    /// Whether the error details are expanded.
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    fn apply_update(&mut self, update: &SessionUpdate, cx: &mut Context<Self>) {
        if update.cluster != self.cluster {
            return;
        }
        match update.change {
            // What the view shows follows these; the rest (namespaces, capabilities) does not.
            SessionChange::StateChanged { .. }
            | SessionChange::DisplayNameChanged(_)
            | SessionChange::Closed
            | SessionChange::Opened => self.sync(cx),
            _ => {}
        }
    }

    /// Reads the session again and redraws when what the view shows changed. A state change is
    /// rare and the user is waiting for it, so it redraws at once: only the view of the cluster
    /// that changed does any work.
    pub(super) fn sync(&mut self, cx: &mut Context<Self>) {
        let session = self.deps.sessions.get(&self.cluster);
        let changed = match session {
            Some(session) => self.follow(&session),
            // The session is gone (its tab closes with it): show nothing of a state it no
            // longer has.
            None => self.set(ClusterSessionState::Disconnected, self.info.clone()),
        };
        if changed {
            cx.notify();
        }
    }

    fn follow(&mut self, session: &ClusterSession) -> bool {
        let info = ConnectInfo::of(
            session,
            self.deps.open_terminal.is_some(),
            self.deps.open_sources.is_some(),
        );
        self.set(session.state().clone(), info)
    }

    fn set(&mut self, state: ClusterSessionState, info: ConnectInfo) -> bool {
        let model = ConnectViewModel::of(&state, &info);
        self.info = info;
        if self.state == state && self.model == model {
            return false;
        }
        // A new state starts with its details collapsed: the summary first.
        if self.state != state {
            self.details_open = false;
        }
        self.state = state;
        self.model = model;
        true
    }

    /// Sends `cluster::Reconnect`: Retry.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.send(
            Command::ClusterReconnect {
                cluster: self.cluster.clone(),
            },
            cx,
        );
    }

    /// Sends `cluster::CancelConnect`: Cancel.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.send(
            Command::ClusterCancelConnect {
                cluster: self.cluster.clone(),
            },
            cx,
        );
    }

    /// Sends `cluster::Connect`: Connect, for a session that is not connecting.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        self.send(
            Command::ClusterConnect {
                cluster: self.cluster.clone(),
            },
            cx,
        );
    }

    /// Opens a terminal on the cluster's context, when the host offers one.
    pub fn open_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.deps.open_terminal.clone() {
            open(&self.cluster, cx);
        }
    }

    /// Opens the kubeconfig sources, when the host offers them.
    pub fn open_sources(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.deps.open_sources.clone() {
            open(cx);
        }
    }

    /// Expands or collapses the error details.
    pub fn toggle_details(&mut self, cx: &mut Context<Self>) {
        self.details_open = !self.details_open;
        cx.notify();
    }

    /// Copies the error details (redacted) to the clipboard.
    pub fn copy_details(&mut self, cx: &mut Context<Self>) {
        let text = match &self.model {
            ConnectViewModel::Error(error) => error.details.clone(),
            ConnectViewModel::AuthRequired(auth) => auth.message.full.clone(),
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn send(&self, command: Command, cx: &mut Context<Self>) {
        self.deps.dispatcher.clone().dispatch(command, cx);
    }
}

impl Render for ConnectView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        self.render_body(window, cx)
    }
}
