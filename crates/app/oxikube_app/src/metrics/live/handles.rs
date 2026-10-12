//! The two handles a consumer holds: [`MetricsInterest`] (a table or panel that shows a kind) and
//! [`MetricsVisibility`] (the cluster tab a window shows). Dropping either withdraws it.

use std::sync::Weak;

use oxikube_domain::ids::ClusterId;

use super::service::Inner;
use super::state::MetricsKind;

/// A consumer of one kind of one cluster's metrics. Dropping it unregisters the consumer; the
/// last one gone stops the poller (its task is dropped, so the call in flight is aborted).
#[must_use = "the cluster is polled only while the interest is held"]
pub struct MetricsInterest {
    inner: Weak<Inner>,
    cluster: ClusterId,
    kind: MetricsKind,
}

impl MetricsInterest {
    pub(super) fn new(inner: Weak<Inner>, cluster: ClusterId, kind: MetricsKind) -> Self {
        Self {
            inner,
            cluster,
            kind,
        }
    }
}

impl std::fmt::Debug for MetricsInterest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsInterest")
            .field("cluster", &self.cluster)
            .field("kind", &self.kind)
            .finish()
    }
}

impl Drop for MetricsInterest {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        if let Some(entry) = inner.state.lock().clusters.get_mut(&self.cluster) {
            let count = &mut entry.interest[self.kind.index()];
            *count = count.saturating_sub(1);
        }
        inner.reconcile(&self.cluster);
    }
}

/// Which cluster tab one window shows. See
/// [`MetricsService::visibility`](super::MetricsService::visibility).
pub struct MetricsVisibility {
    inner: Weak<Inner>,
    shown: Option<ClusterId>,
}

impl std::fmt::Debug for MetricsVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsVisibility")
            .field("shown", &self.shown)
            .finish()
    }
}

impl MetricsVisibility {
    pub(super) fn new(inner: Weak<Inner>) -> Self {
        Self { inner, shown: None }
    }

    /// The window now shows `cluster`'s tab (`None`: no cluster tab, the catalog say).
    pub fn show(&mut self, cluster: Option<ClusterId>) {
        if self.shown == cluster {
            return;
        }
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let previous = std::mem::replace(&mut self.shown, cluster.clone());
        {
            let mut state = inner.state.lock();
            if let Some(entry) = previous.as_ref().and_then(|c| state.clusters.get_mut(c)) {
                entry.visible = entry.visible.saturating_sub(1);
            }
            if let Some(cluster) = &cluster {
                state.entry(cluster).visible += 1;
            }
        }
        for cluster in previous.iter().chain(cluster.iter()) {
            inner.reconcile(cluster);
        }
    }
}

impl Drop for MetricsVisibility {
    fn drop(&mut self) {
        self.show(None);
    }
}
