//! The overview's warnings panel over the service: [`RecentWarnings`] for [`EventService`].

use oxikube_domain::ids::ClusterId;

use super::service::EventService;
use crate::overview::{RecentWarnings, WarningRow};

impl RecentWarnings for EventService {
    /// The newest `n` `Warning` events the cluster's buffer holds, read off the warnings index
    /// (no scan of the other events). Empty while nobody subscribes to the cluster's events.
    fn latest(&self, cluster: &ClusterId, n: usize) -> Vec<WarningRow> {
        self.inner
            .read(cluster, |buffer| {
                buffer
                    .warnings()
                    .take(n)
                    .map(|event| WarningRow {
                        namespace: event.regarding.namespace.clone(),
                        object: format!("{}/{}", event.regarding.gvk.kind, event.regarding.name),
                        reason: event.reason.to_string(),
                        message: event.message.clone(),
                        count: event.count.max(1),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}
