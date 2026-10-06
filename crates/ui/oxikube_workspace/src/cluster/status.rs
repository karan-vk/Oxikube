//! [`ClusterStatusItem`]: the active cluster's name, colour and read-only badge in the status bar.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window, div, prelude::*,
};
use oxikube_app::{ClusterSession, ClusterSessionManager};
use oxikube_domain::ids::ClusterId;
use oxikube_ui::{ActiveTokens as _, layout::h_flex, u};

use super::follow::follow_session;
use super::mark::{BadgeSurface, ClusterBadge, ClusterMark};
use crate::motion::animation_duration;
use crate::status_bar::StatusItem;

/// How long the denial pulse runs (clamped by the app-wide animation cap, off under
/// reduce-motion).
const PULSE: Duration = Duration::from_millis(150);

/// The status bar's cluster badge.
///
/// Shows the active cluster's name with its colour dot and, when read-only, the lock and the
/// words "Read-only". It follows the session itself ([`follow_session`]): a change of the flag
/// or the colour redraws just this item, once. Hidden while no cluster is active. Register it
/// with [`Workspace::register_status_item`](crate::Workspace::register_status_item) on the left
/// side; the workspace sets the active cluster with [`set_cluster`](Self::set_cluster).
pub struct ClusterStatusItem {
    manager: ClusterSessionManager,
    cluster: Option<ClusterId>,
    mark: ClusterMark,
    title: SharedString,
    /// How many pulses ran; keys the animation so each one plays once.
    pulses: usize,
    pulsing: bool,
    _follow: Task<()>,
}

impl ClusterStatusItem {
    /// An item with no active cluster, following `manager`.
    pub fn new(manager: ClusterSessionManager, cx: &mut Context<Self>) -> Self {
        let follow = follow_session(
            &manager,
            cx,
            |this: &Self| this.cluster.clone(),
            |this, session, cx| {
                this.show(session);
                cx.notify();
            },
        );
        Self {
            manager,
            cluster: None,
            mark: ClusterMark::default(),
            title: SharedString::default(),
            pulses: 0,
            pulsing: false,
            _follow: follow,
        }
    }

    /// Makes `cluster` the one shown (`None` hides the item).
    pub fn set_cluster(&mut self, cluster: Option<ClusterId>, cx: &mut Context<Self>) {
        let session = cluster.as_ref().and_then(|id| self.manager.get(id));
        self.cluster = cluster;
        self.show(session);
        cx.notify();
    }

    fn show(&mut self, session: Option<ClusterSession>) {
        match session {
            Some(session) => {
                self.mark = ClusterMark::of(&session);
                self.title = session.title().to_owned().into();
            }
            None => {
                self.mark = ClusterMark::default();
                self.title = SharedString::default();
            }
        }
    }

    /// The cluster shown.
    pub fn cluster(&self) -> Option<&ClusterId> {
        self.cluster.as_ref()
    }

    /// The mark drawn now.
    pub fn mark(&self) -> ClusterMark {
        self.mark
    }

    /// How many denial pulses have started (for tests and diagnostics).
    pub fn pulse_count(&self) -> usize {
        self.pulses
    }

    /// Draws attention to the badge once, after a mutation was refused on `cluster`.
    ///
    /// Does nothing for another cluster, for a cluster that is not read-only, while a pulse is
    /// still running (a burst of refusals pulses once) or under reduce-motion (the toast carries
    /// the message). The pulse is one short fade, never a loop.
    pub fn pulse(&mut self, cluster: &ClusterId, cx: &mut Context<Self>) {
        if self.cluster.as_ref() != Some(cluster) || !self.mark.read_only || self.pulsing {
            return;
        }
        let Some(duration) = animation_duration(cx, PULSE) else {
            return;
        };
        self.pulsing = true;
        self.pulses += 1;
        cx.notify();
        // Detached on purpose: the timer only clears a flag, nothing stores or drops it.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            this.update(cx, |this, cx| {
                this.pulsing = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl StatusItem for ClusterStatusItem {
    fn visible(&self, _: &App) -> bool {
        self.cluster.is_some()
    }
}

impl Render for ClusterStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let warning = tokens.colors.warning;
        let item = h_flex()
            .id(("status-cluster", self.pulses))
            .debug_selector(|| "status-cluster".to_owned())
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.sm))
            .rounded(u(tokens.radius.sm))
            .items_center()
            .child(
                ClusterBadge::new(self.mark, BadgeSurface::StatusBar).name("status-cluster-badge"),
            )
            .child(self.title.clone())
            .when(self.mark.read_only, |this| {
                this.child(
                    div()
                        .debug_selector(|| "status-cluster-read-only".to_owned())
                        .child("Read-only"),
                )
            });
        match animation_duration(cx, PULSE).filter(|_| self.pulsing) {
            Some(duration) => item
                .with_animation(
                    ("status-cluster-pulse", self.pulses),
                    Animation::new(duration),
                    move |this, delta| this.bg(warning.opacity(0.45 * (1.0 - delta))),
                )
                .into_any_element(),
            None => item.into_any_element(),
        }
    }
}
