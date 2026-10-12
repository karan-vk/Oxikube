//! [`NodePanel`]: the node detail panel above the generic Overview.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | the entity: what it follows, the one-second tick, the actions |
//! | `data` | the derived view-models and rows ([`NodeData`](data::NodeData), [`PodsOnNode`](data::PodsOnNode)) |
//! | `refresh` | following the node, the pods-by-node index and the one-time list |
//! | `render` | the frame: the cordon badge and button, the allocation section |
//! | `sections` | conditions, taints, capacity, system info |
//! | `lists` | the pods and the images, virtualised |
//! | `allocation` | the allocation section (E13-S11): the view built from the pods, the usage it polls for |
//!
//! The panel keeps a lease on the pods-by-node index while the detail is open (releasing it with
//! the detail) and reads it once a second, or when the node changes, never from `render`.

mod allocation;
mod data;
mod lists;
mod refresh;
mod render;
mod sections;

use std::time::Duration;

use gpui::{
    Context, IntoElement, Render, Subscription, Task, UniformListScrollHandle, WeakEntity, Window,
};
use oxikube_app::allocation::{AllocationView, UsageState};
use oxikube_app::store::{NodeLoadsLease, ResourceStore};
use oxikube_domain::Command;
use oxikube_settings::{MetricsSettings, Settings as _};

use self::data::{Followed, NodeData, PodsOnNode};
use crate::detail::DetailView;
use crate::kinds::allocation::poll_usage;

/// How often the panel looks at the pods on its node while it has none to show (the Pod feed is
/// still listing): the first answer appears within a quarter of a second of the feed's.
const TICK_WAITING: Duration = Duration::from_millis(250);
/// How often it looks once it has them: the pods of a node change at human speed, and a look is
/// one hash lookup and a pointer comparison per pod, so a second is plenty.
const TICK: Duration = Duration::from_secs(1);

/// Where the pods come from when the live index is not there.
pub(super) enum Fetch {
    /// Nothing asked yet.
    Idle,
    /// The one-time list is running.
    Loading(#[allow(dead_code)] Task<()>),
    /// It answered (its rows are in the panel's pods).
    Done,
    /// It failed: why.
    Failed(String),
}

/// The node panel. See the `kinds::node::panel` module docs.
pub struct NodePanel {
    detail: WeakEntity<DetailView>,
    node: Option<NodeData>,
    store: Option<ResourceStore>,
    lease: Option<NodeLoadsLease>,
    pods: Option<PodsOnNode>,
    /// What the pod rows were built from (see [`Followed`](data::Followed)).
    followed: Option<Followed>,
    /// Ticks the live index was not ready for (the fallback list starts after two).
    waited: u32,
    fetch: Fetch,
    /// Why cordon and uncordon are blocked now (read-only), `None` when they are allowed.
    blocked: Option<String>,
    pods_scroll: UniformListScrollHandle,
    images_scroll: UniformListScrollHandle,
    /// The allocation section, rebuilt when the node, its pods, the usage or the GPU list change.
    view: Option<AllocationView>,
    /// The last answer of the metrics port.
    usage: UsageState,
    _follow: Option<Subscription>,
    _tick: Task<()>,
    /// Polls the metrics port while the panel is open (dropped with it).
    _usage: Task<()>,
    /// Rebuilds the allocation when `metrics.gpu_vendors` changes.
    _gpus: Subscription,
}

impl NodePanel {
    /// The panel of the node the `detail` shows.
    pub fn new(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let follow = detail
            .upgrade()
            .map(|view| cx.observe(&view, |this, _, cx| this.refresh(cx)));
        let tick = cx.spawn(async move |this, cx| {
            let mut wait = TICK_WAITING;
            loop {
                cx.background_executor().timer(wait).await;
                let next = this.update(cx, |panel, cx| {
                    panel.refresh(cx);
                    if panel.pods.is_some() {
                        TICK
                    } else {
                        TICK_WAITING
                    }
                });
                match next {
                    Ok(next) => wait = next,
                    Err(_) => break,
                }
            }
        });
        let usage = poll_usage(
            cx,
            |panel: &Self, cx| panel.usage_target(cx),
            |panel, state, cx| panel.usage_arrived(state, cx),
        );
        let gpus = MetricsSettings::observe_in(cx, |panel: &mut Self, cx| {
            panel.rebuild_view(cx);
            cx.notify();
        });
        // Built while the detail draws (it is leased then, so it cannot be read here): the first
        // look is the next thing the foreground does.
        cx.spawn(async move |this, cx| {
            this.update(cx, |panel, cx| panel.refresh(cx)).ok();
        })
        .detach();
        Self {
            detail,
            node: None,
            store: None,
            lease: None,
            pods: None,
            followed: None,
            waited: 0,
            fetch: Fetch::Idle,
            blocked: None,
            pods_scroll: UniformListScrollHandle::new(),
            images_scroll: UniformListScrollHandle::new(),
            view: None,
            usage: UsageState::Pending,
            _follow: follow,
            _tick: tick,
            _usage: usage,
            _gpus: gpus,
        }
    }

    /// Cordons the node, or uncordons it when it is cordoned: sends `node::Cordon` or
    /// `node::Uncordon`, which the bus confirms, guards and audits.
    pub fn toggle_cordon(&mut self, cx: &mut Context<Self>) {
        let (Some(node), Some(detail)) = (&self.node, self.detail.upgrade()) else {
            return;
        };
        if self.blocked.is_some() {
            return;
        }
        let (target, dispatcher) = {
            let view = detail.read(cx);
            (view.target().clone(), view.deps().dispatcher.clone())
        };
        let command = if node.summary.schedulable {
            Command::NodeCordon { target }
        } else {
            Command::NodeUncordon { target }
        };
        dispatcher.dispatch(command, cx);
    }

    /// Whether the panel offers "Drain…": the app gave a drain dialog.
    pub(crate) fn offers_drain(&self, cx: &gpui::App) -> bool {
        self.detail
            .upgrade()
            .is_some_and(|detail| detail.read(cx).deps().drain.is_some())
    }

    /// Opens the drain dialog for the node (the same dialog as the table's "Drain…" and `r`). Does
    /// nothing while drains are blocked (read-only) or without a dialog to open.
    pub fn open_drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked.is_some() {
            return;
        }
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (opener, dispatcher, target) = {
            let view = detail.read(cx);
            let deps = view.deps();
            (
                deps.drain.clone(),
                deps.dispatcher.clone(),
                view.target().clone(),
            )
        };
        if let Some(opener) = opener {
            opener.open(dispatcher, target, window, cx);
        }
    }
}

impl Render for NodePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.body(cx)
    }
}

/// The panel as a typed entity, for tests.
#[cfg(test)]
pub(crate) fn of(view: &gpui::AnyView) -> gpui::Entity<NodePanel> {
    view.clone().downcast::<NodePanel>().expect("a node panel")
}

/// What the tests look at: the panel's state, not its pixels.
#[cfg(test)]
impl NodePanel {
    /// The allocation section as the panel last built it.
    pub(crate) fn allocation_view(&self) -> Option<&AllocationView> {
        self.view.as_ref()
    }

    /// `namespace/name` of the pods listed, in order.
    pub(crate) fn pod_names(&self) -> Vec<String> {
        self.pods
            .iter()
            .flat_map(|pods| &pods.rows)
            .map(|row| format!("{}/{}", row.namespace, row.name))
            .collect()
    }

    /// `namespace/name status` of the pods listed, in order.
    pub(crate) fn pod_statuses(&self) -> Vec<String> {
        self.pods
            .iter()
            .flat_map(|pods| &pods.rows)
            .map(|row| format!("{}/{} {}", row.namespace, row.name, row.status))
            .collect()
    }

    /// Whether the pods came from the live index (`false`: the one-time list), `None` without
    /// pods.
    pub(crate) fn pods_are_live(&self) -> Option<bool> {
        self.pods.as_ref().map(|pods| pods.live)
    }

    /// Why the one-time list failed, if it did.
    pub(crate) fn fetch_error(&self) -> Option<&str> {
        match &self.fetch {
            Fetch::Failed(why) => Some(why),
            _ => None,
        }
    }

    /// Why the cordon button is blocked, if it is.
    pub(crate) fn blocked(&self) -> Option<&str> {
        self.blocked.as_deref()
    }
}
