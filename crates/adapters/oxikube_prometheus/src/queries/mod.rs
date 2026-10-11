//! The query catalogue (E13-S06): the PromQL for each scope and metric on each stack.
//!
//! [`PromqlCatalogue`] implements [`QueryCataloguePort`]. It does no I/O: a
//! [`MetricRequest`] and a [`MetricsProviderKind`] go in, a query string comes out, so every
//! combination is a plain unit test.
//!
//! # Shape
//!
//! Series shared by every stack are built once, by the family that owns them; a stack's module
//! ([`kube_prometheus`], [`lens`], [`victoria`], [`mimir`], [`openshift`], [`prometheus`])
//! holds its [`Profile`] and documents the conventions it was written against, so a reviewer can
//! compare them with the real thing.
//!
//! | Scope | cpu, memory | requests, limits | network | disk | pod count |
//! |---|---|---|---|---|---|
//! | cluster, node | node-exporter | kube-state-metrics | node-exporter | node-exporter | kube-state-metrics |
//! | namespace | cAdvisor | kube-state-metrics | cAdvisor | kubelet volume stats | kube-state-metrics |
//! | pod, container | cAdvisor | kube-state-metrics | pod: cAdvisor | cAdvisor writable layer | - |
//! | PVC | - | - | - | kubelet volume stats | - |
//! | Ingress | - | - | ingress-nginx or HAProxy router | - | - |
//!
//! * Pod targets return one series per pod (`sum by (pod)`), container targets one per container
//!   (`sum by (container)`); everything else is one unlabelled series.
//! * Nodes are picked through `node_uname_info{nodename}`, not `instance`, which holds a node
//!   name on some stacks and `ip:9100` on others.
//! * Selectors only: every query names its namespace, pod, node or claim, so cost does not grow
//!   with the cluster. No regex over everything; a set of pods is an anchored alternation of
//!   escaped names.
//! * Every name is escaped on its way in (see [`selector`]).
//! * Nothing here talks to a cluster; running the query is `PromqlPort`'s job (ADR 0011).

mod ctx;
mod host;
mod pods;
mod profile;
mod selector;
mod storage;
mod traffic;
mod workload;

mod kube_prometheus;
mod lens;
mod mimir;
mod openshift;
mod prometheus;
mod victoria;

use std::time::Duration;

use oxikube_domain::metric_query::promql_duration;
use oxikube_domain::{
    MetricKind, MetricQuery, MetricQueryError, MetricRequest, MetricScope, MetricsProviderKind,
};
use oxikube_ports::QueryCataloguePort;

use ctx::Ctx;
use profile::Profile;

/// The query catalogue of every [`MetricsProviderKind`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PromqlCatalogue;

impl PromqlCatalogue {
    fn profile(provider: MetricsProviderKind) -> &'static Profile {
        match provider {
            MetricsProviderKind::KubePrometheus => &kube_prometheus::PROFILE,
            MetricsProviderKind::Lens => &lens::PROFILE,
            MetricsProviderKind::VictoriaMetrics => &victoria::PROFILE,
            MetricsProviderKind::Mimir => &mimir::PROFILE,
            MetricsProviderKind::OpenShift => &openshift::PROFILE,
            MetricsProviderKind::Prometheus => &prometheus::PROFILE,
        }
    }

    /// Whether the store of `provider` usually holds several clusters, so a query should carry
    /// a cluster matcher (`MetricRequest::with_label`) to avoid mixing them.
    pub fn is_shared_store(provider: MetricsProviderKind) -> bool {
        Self::profile(provider).shared_store
    }
}

impl QueryCataloguePort for PromqlCatalogue {
    fn build(
        &self,
        provider: MetricsProviderKind,
        request: &MetricRequest,
        rate_window: Duration,
    ) -> Result<MetricQuery, MetricQueryError> {
        request.validate()?;
        if rate_window.as_secs() == 0 {
            return Err(MetricQueryError::EmptyWindow);
        }
        let profile = Self::profile(provider);
        let ctx = Ctx {
            profile,
            request,
            window: promql_duration(rate_window),
        };
        let promql = match request.kind {
            MetricKind::CpuUsage | MetricKind::MemoryUsage => match request.scope() {
                MetricScope::Cluster | MetricScope::Node => host::build(&ctx),
                _ => workload::usage(&ctx),
            },
            MetricKind::CpuRequests
            | MetricKind::CpuLimits
            | MetricKind::MemoryRequests
            | MetricKind::MemoryLimits => workload::reference(&ctx),
            MetricKind::NetworkRx | MetricKind::NetworkTx => traffic::build(&ctx),
            MetricKind::DiskUsage | MetricKind::DiskCapacity => storage::build(&ctx),
            MetricKind::PodCount => pods::build(&ctx),
        }?;
        Ok(MetricQuery {
            promql,
            series_label: request.target.series_label(),
            unit: request.kind.unit(),
        })
    }
}
