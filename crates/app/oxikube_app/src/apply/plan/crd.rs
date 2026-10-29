//! Waiting for a new CustomResourceDefinition: the server serves its kind only once the definition
//! is `Established`, and the cluster's discovery knows the kind only after a refresh. Without the
//! wait the first custom resource races the definition and fails with "no matches for kind".

use std::collections::HashSet;
use std::time::Duration;

use oxikube_domain::ids::ResourceRef;
use serde_json::Value;

use super::item::Item;
use super::run::Run;

/// How long a new definition may take to become `Established`.
pub(super) const ESTABLISHED_TIMEOUT: Duration = Duration::from_secs(30);

/// How often its conditions are read meanwhile.
pub(super) const ESTABLISHED_POLL: Duration = Duration::from_millis(500);

impl Run<'_> {
    /// Waits until each of `crds` (applied items) is `Established`, then refreshes discovery once.
    /// Returns the ones that were not established in time: their custom resources are skipped.
    pub(super) async fn settle_crds(&self, crds: &[&Item]) -> HashSet<usize> {
        let mut late = HashSet::new();
        for crd in crds {
            let Some(target) = crd.target() else {
                continue;
            };
            if !self.wait_established(target).await {
                tracing::warn!(
                    crd = %target.name,
                    "the CustomResourceDefinition was not Established in time"
                );
                late.insert(crd.index);
            }
        }
        self.refresh_discovery().await;
        late
    }

    /// Re-runs discovery, so the cluster's kinds include what the plan's definitions added (the
    /// adapter reports the change to every view).
    pub(super) async fn refresh_discovery(&self) {
        if let Some(discovery) = self.discovery
            && let Err(error) = discovery.discover().await
        {
            // The custom resources' own requests may still find the kind; their errors say so.
            tracing::warn!(error = %error, "could not refresh discovery after a new definition");
        }
    }

    /// Polls `crd` until its `Established` condition is `True`, for at most
    /// [`ESTABLISHED_TIMEOUT`]. `false` when it never was.
    async fn wait_established(&self, crd: &ResourceRef) -> bool {
        let mut waited = Duration::ZERO;
        loop {
            if let Ok(Some(object)) = self.reader.get_opt(&crd.gvk, None, &crd.name).await
                && is_established(&object.to_value())
            {
                return true;
            }
            if waited >= ESTABLISHED_TIMEOUT {
                return false;
            }
            self.clock.sleep(ESTABLISHED_POLL).await;
            waited += ESTABLISHED_POLL;
        }
    }
}

/// Whether the definition's `status.conditions` say `Established: True`.
pub(super) fn is_established(crd: &Value) -> bool {
    crd.pointer("/status/conditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|condition| {
            condition.get("type").and_then(Value::as_str) == Some("Established")
                && condition.get("status").and_then(Value::as_str) == Some("True")
        })
}
