//! [`TableMetrics`]: a resource table's side of live metrics.

use gpui::{App, SharedString};
use oxikube_app::{MetricsInterest, MetricsKind};
use oxikube_domain::ids::{ClusterId, Gvk};

/// What a resource table watches and the badge it shows.
///
/// A Pods or Nodes table watches its kind while it is the active item of its pane (the tab you
/// see); any other table watches nothing. The cluster's poller runs only while something
/// watches and the cluster's tab is shown, so a table in a background tab or pane costs no
/// polls.
#[derive(Debug, Default)]
pub struct TableMetrics {
    kind: Option<MetricsKind>,
    interest: Option<MetricsInterest>,
    /// The badge text the last frame drew.
    shown: Option<SharedString>,
}

impl TableMetrics {
    /// The metrics of a table of `gvk`: Pods and Nodes have usage columns, other kinds none.
    pub fn for_kind(gvk: &Gvk) -> Self {
        let kind = match (gvk.group.as_ref(), gvk.kind.as_ref()) {
            ("", "Pod") => Some(MetricsKind::Pods),
            ("", "Node") => Some(MetricsKind::Nodes),
            _ => None,
        };
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Whether the table holds an interest now.
    pub fn is_watching(&self) -> bool {
        self.interest.is_some()
    }

    /// Watches while `active`, stops otherwise.
    pub fn set_active(&mut self, active: bool, cluster: &ClusterId, cx: &App) {
        match (active, self.kind, &self.interest) {
            (true, Some(kind), None) => {
                self.interest = super::service(cx).map(|service| service.watch(cluster, kind));
            }
            (false, _, Some(_)) => self.interest = None,
            _ => {}
        }
    }

    /// Stops watching (the table closed).
    pub fn release(&mut self) {
        self.interest = None;
    }

    /// `metrics unavailable: <reason>` for the table's kind in `cluster`, or `None` while metrics
    /// are there (or the table has none).
    pub fn badge_text(&self, cluster: &ClusterId, cx: &App) -> Option<SharedString> {
        let kind = self.kind?;
        let state = super::service(cx)?.state(cluster, kind);
        state.unavailable_text().map(SharedString::from)
    }

    /// Records the badge text a frame draws.
    pub fn drew(&mut self, text: Option<SharedString>) {
        self.shown = text;
    }

    /// Whether the badge would read differently from what the last frame drew (the table's
    /// once-a-second check redraws on `true`).
    pub fn moved(&self, cluster: &ClusterId, cx: &App) -> bool {
        self.kind.is_some() && self.badge_text(cluster, cx) != self.shown
    }
}
