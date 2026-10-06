//! [`KubeAccess`]: `AccessReviewPort` on the rules review plus discovery.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{Capabilities, OxiResult};
use oxikube_ports::AccessReviewPort;

use super::REFRESH;
use crate::discovery::KubeDiscovery;
use crate::health::{RulesCache, capabilities_for_context};
use crate::pool::ClientPool;

/// The namespace a cluster-wide question is asked in: the rules review needs one, and the
/// cluster-scoped grants show up in every namespace's answer.
const CLUSTER_WIDE_PROBE_NAMESPACE: &str = "default";

/// The API group `metrics-server` serves; its presence is the `METRICS` capability.
const METRICS_GROUP: &str = "metrics.k8s.io";

/// What the current user may do on one connection.
///
/// RBAC flags (`MUTATE`, `EXEC`, `LOGS`, `PORTFORWARD`) come from a `SelfSubjectRulesReview`
/// (granted or restricted to named objects: the action is offered and the server decides per
/// object). `METRICS` comes from discovery. `HELM` and `ARGO` are not Kubernetes API facts; their
/// adapters add them.
pub(super) struct KubeAccess {
    pool: Arc<ClientPool>,
    rules: Arc<RulesCache>,
    context: ContextName,
    discovery: KubeDiscovery,
}

impl KubeAccess {
    pub(super) fn new(
        pool: Arc<ClientPool>,
        rules: Arc<RulesCache>,
        context: ContextName,
        discovery: KubeDiscovery,
    ) -> Self {
        Self {
            pool,
            rules,
            context,
            discovery,
        }
    }
}

#[async_trait]
impl AccessReviewPort for KubeAccess {
    async fn capabilities(&self, namespace: Option<&str>) -> OxiResult<Capabilities> {
        let namespace = namespace.unwrap_or(CLUSTER_WIDE_PROBE_NAMESPACE);
        let report =
            capabilities_for_context(&self.pool, &self.rules, &self.context, namespace, REFRESH)
                .await?;
        let mut capabilities = report.available();
        // Waits for the discovery the manager runs beside this call (or runs it), so the flag
        // does not depend on which of the two finishes first.
        if self.discovery.serves_group(METRICS_GROUP).await {
            capabilities |= Capabilities::METRICS;
        }
        Ok(capabilities)
    }
}
