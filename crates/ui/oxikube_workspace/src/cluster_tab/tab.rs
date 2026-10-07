//! [`ClusterTab`]: one connected cluster as a tab of the window's workspace.
//!
//! A cluster tab is an [`Item`] that hosts a whole [`Workspace`] of its own (the "per-cluster
//! Workspace entity inside a tab" shape of the story, chosen over a bare item with panels because
//! every later cluster story then plugs in through the workspace API it already knows):
//!
//! - its own **docks**, so the sidebar panel (E06-S10) sits in the cluster's left dock and each
//!   cluster keeps its own dock sizes and open flags;
//! - its own **pane group** of items (resource lists, logs, editors: E07 and later), so splits,
//!   tabs and focus are independent between clusters;
//! - its own **layout persistence**, saved under the cluster's id and restored when the cluster's
//!   tab opens again (see [`ClusterTabs`](super::ClusterTabs)).
//!
//! The inner workspace [embeds](Workspace::embedded) in the window's: the status bar, modal layer
//! and toast layer are the window's, drawn once.
//!
//! Only the displayed tab is rendered (the dock area draws one tab per pane), and
//! [`Item::set_active`] tells the tab when it is shown or hidden, which becomes
//! [`ClusterTabEvent::ActiveChanged`] for whoever owns the cluster's feeds (the watch budget,
//! metrics polling): hidden clusters do not render, and consumers pause what they poll.

use gpui::{
    AnyView, App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{ClusterSessionState, SessionPhase};
use oxikube_ui::{ActiveTokens as _, layout::v_flex, u};

use super::colour::cluster_hsla;
use crate::{
    cluster::ClusterMark,
    item::{CloseRequest, Item, ItemEvent, TabContent},
    persistence::LayoutPersistence,
    workspace::Workspace,
};

/// What a cluster tab shows about its cluster. Kept in step with the session by
/// [`ClusterTabs`](super::ClusterTabs).
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterTabInfo {
    /// The tab title: the cluster's display name, else its context name.
    pub title: SharedString,
    /// The cluster's colour and read-only lock: the tab's dot, lock and stripe.
    pub mark: ClusterMark,
    /// The session's connection state: the tab shows its [`ConnectUi`] (or, without one, a
    /// placeholder) while the cluster is not connected.
    pub state: ClusterSessionState,
}

/// The views a cluster tab shows while its session is connecting or has trouble (the connect
/// lifecycle, E06-S06). They come from `oxikube_catalog_ui::connect`, which this crate cannot
/// depend on, so the owner hands them in with [`ClusterTab::set_connect_ui`].
///
/// - `body` replaces the tab's content while the session is `Connecting`, `AuthRequired`,
///   `Error` or `Disconnected`: the cluster's views cannot show anything true then.
/// - `banner` is drawn above the content while the session is `Degraded`: the content stays.
#[derive(Clone)]
pub struct ConnectUi {
    /// The full-body view of a session that is not connected.
    pub body: AnyView,
    /// The banner of a degraded session.
    pub banner: AnyView,
}

/// What a cluster tab tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClusterTabEvent {
    /// The tab became the displayed one (`true`) or was hidden (`false`).
    ActiveChanged(bool),
    /// The user asked to close the tab (its close button, `workspace::CloseActiveItem` with an
    /// empty cluster workspace): the owner confirms and disconnects.
    CloseRequested,
    /// The tab left the workspace.
    Closed,
}

/// The tab of one cluster. See the module docs.
pub struct ClusterTab {
    cluster: ClusterId,
    info: ClusterTabInfo,
    workspace: Entity<Workspace>,
    /// Saves and restores the inner layout; owned by the inner workspace too, held here for
    /// the flush when the tab closes.
    persistence: Option<Entity<LayoutPersistence>>,
    connect_ui: Option<ConnectUi>,
    /// The strip above the content of a connected cluster (the namespace selector), set by the
    /// owner.
    toolbar: Option<AnyView>,
    active: bool,
    _observe_workspace: Subscription,
}

impl EventEmitter<ItemEvent> for ClusterTab {}
impl EventEmitter<ClusterTabEvent> for ClusterTab {}

impl ClusterTab {
    /// A tab for `cluster` hosting `workspace` (the cluster's own, [embedded](Workspace::embedded)
    /// in the window's).
    pub fn new(
        cluster: ClusterId,
        info: ClusterTabInfo,
        workspace: Entity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Whatever the cluster's workspace does (an item opens, the restore lands) may change
        // what the tab draws: the placeholder gives way to the real layout.
        let observe = cx.observe(&workspace, |_, _, cx| cx.notify());
        Self {
            cluster,
            info,
            workspace,
            persistence: None,
            connect_ui: None,
            toolbar: None,
            active: false,
            _observe_workspace: observe,
        }
    }

    /// The cluster this tab shows.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The cluster's own workspace: add its sidebar panel, open its items.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// What the tab shows about its cluster.
    pub fn info(&self) -> &ClusterTabInfo {
        &self.info
    }

    /// Whether the tab is the displayed one of its pane. A hidden cluster must not render and
    /// should pause what it polls.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Replaces what the tab shows. Redraws the tab label and body when it changed.
    pub fn set_info(&mut self, info: ClusterTabInfo, cx: &mut Context<Self>) {
        if self.info != info {
            self.info = info;
            cx.emit(ItemEvent::UpdateTab);
            cx.notify();
        }
    }

    /// Shows `ui` for the session's connect lifecycle instead of the placeholder.
    pub fn set_connect_ui(&mut self, ui: ConnectUi, cx: &mut Context<Self>) {
        self.connect_ui = Some(ui);
        cx.notify();
    }

    /// The connect views the tab shows while its cluster is not connected, if its owner set
    /// them.
    pub fn connect_ui(&self) -> Option<&ConnectUi> {
        self.connect_ui.as_ref()
    }

    /// Shows `toolbar` above the content while the cluster is connected (`Ready` or
    /// `Degraded`): the namespace selector (E06-S07) and, later, the view's own controls. It
    /// comes from crates this one cannot depend on, so the owner hands it in.
    pub fn set_toolbar(&mut self, toolbar: AnyView, cx: &mut Context<Self>) {
        self.toolbar = Some(toolbar);
        cx.notify();
    }

    /// The toolbar the tab shows while connected, if its owner set one.
    pub fn toolbar(&self) -> Option<&AnyView> {
        self.toolbar.as_ref()
    }

    pub(super) fn set_persistence(&mut self, persistence: Entity<LayoutPersistence>) {
        self.persistence = Some(persistence);
    }
}

impl Focusable for ClusterTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.workspace.read(cx).focus_handle(cx)
    }
}

impl Item for ClusterTab {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.info.title.clone()).cluster(self.info.mark)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(format!("cluster:{}", self.cluster).into())
    }

    fn intercepts_close(&self, _: &App) -> bool {
        true
    }

    fn close_requested(&mut self, _: &mut Window, cx: &mut Context<Self>) -> CloseRequest {
        // Emitted, not handled here: the owner opens the confirmation once this update ends.
        cx.emit(ClusterTabEvent::CloseRequested);
        CloseRequest::Deferred
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            cx.emit(ClusterTabEvent::ActiveChanged(active));
        }
    }

    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = false;
        // Whatever the user changed in the last half second still reaches the state store: the
        // write is its own task, so it outlives the workspace that is about to go.
        if let Some(persistence) = self.persistence.take() {
            persistence
                .update(cx, |persistence, cx| persistence.flush(window, cx))
                .detach();
        }
        cx.emit(ClusterTabEvent::Closed);
    }
}

impl Render for ClusterTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.colors();
        let title = &self.info.title;
        let blank = self.workspace.read(cx).is_blank();
        let phase = self.info.state.phase();
        let content = {
            let content = div().flex_1().min_h_0().w_full();
            if blank {
                content.child(placeholder(&self.info, cx))
            } else {
                content.child(self.workspace.clone())
            }
        };
        let toolbar = self.toolbar.as_ref().filter(|_| phase.is_connected());
        let content = v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .when_some(toolbar, |this, toolbar| {
                this.child(
                    div()
                        .id("cluster-toolbar")
                        .debug_selector(|| format!("cluster-toolbar-{title}"))
                        .flex_none()
                        .w_full()
                        .border_b_1()
                        .border_color(colors.border_variant)
                        .child(toolbar.clone()),
                )
            })
            .child(content);
        let body = match &self.connect_ui {
            Some(ui) if !phase.is_connected() => div()
                .id("cluster-connect")
                .debug_selector(|| format!("cluster-connect-{title}"))
                .flex_1()
                .min_h_0()
                .w_full()
                .child(ui.body.clone())
                .into_any_element(),
            Some(ui) if phase == SessionPhase::Degraded => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(div().flex_none().w_full().child(ui.banner.clone()))
                .child(content)
                .into_any_element(),
            _ => content.into_any_element(),
        };
        v_flex()
            .id("cluster-tab")
            .debug_selector(|| format!("cluster-tab-{title}"))
            .size_full()
            .bg(colors.background)
            .child(
                div()
                    .id("cluster-stripe")
                    .debug_selector(|| format!("cluster-stripe-{title}"))
                    .flex_none()
                    .h(u(px(2.)))
                    .w_full()
                    .when_some(self.info.mark.colour, |this, colour| {
                        this.bg(cluster_hsla(colour))
                    }),
            )
            .child(body)
    }
}

/// The body of a cluster whose workspace has nothing in it yet: the cluster and what its session
/// is doing. Real content (the sidebar, the first views) replaces it as later stories land.
fn placeholder(info: &ClusterTabInfo, cx: &App) -> impl IntoElement {
    let colors = cx.colors();
    let status = status_text(&info.state);
    let phase = format!("{:?}", info.state.phase()).to_lowercase();
    v_flex()
        .id("cluster-placeholder")
        .debug_selector(|| format!("cluster-placeholder-{}", info.title))
        .size_full()
        .items_center()
        .justify_center()
        .gap(u(px(6.)))
        .child(
            div()
                .text_color(colors.text)
                .text_size(u(px(18.)))
                .child(info.title.clone()),
        )
        .child(
            div()
                .text_color(colors.text_muted)
                .text_size(u(px(13.)))
                // Keyed by phase, so a test can tell which state the placeholder draws.
                .debug_selector(|| format!("cluster-placeholder-{phase}-{}", info.title))
                .child(status),
        )
}

/// What a placeholder says about the session `state`.
pub(super) fn status_text(state: &ClusterSessionState) -> String {
    match state {
        ClusterSessionState::Disconnected => "Disconnected".to_owned(),
        ClusterSessionState::Connecting => "Connecting…".to_owned(),
        ClusterSessionState::Ready => "Connected".to_owned(),
        ClusterSessionState::Degraded => "Connected, health checks are failing".to_owned(),
        ClusterSessionState::AuthRequired { reason } => {
            format!("Authentication required: {reason}")
        }
        ClusterSessionState::Error { reason } => format!("Connection failed: {reason}"),
    }
}
