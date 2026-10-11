//! VictoriaMetrics (`victoria-metrics-k8s-stack`, single node or cluster).
//!
//! Assumptions:
//!
//! * vmagent scrapes with the Operator's `VMNodeScrape` / `VMServiceScrape`, which behave like
//!   the Prometheus Operator's: the same cAdvisor, kube-state-metrics v2 and node-exporter
//!   series, and a metric's own `namespace` renamed `exported_namespace` (`honor_labels` is off
//!   by default).
//! * PromQL is a subset of MetricsQL, so these queries run unchanged; the cluster-select path
//!   prefix is part of the endpoint, not of the query.
//! * One store often serves several clusters (vmagent `-remoteWrite.label=cluster=...`): see
//!   [`Profile::shared_store`](super::profile::Profile).

use super::profile::{IngressSource, Profile};

pub(super) const PROFILE: Profile = Profile {
    ingress: IngressSource::Nginx {
        namespace_label: "exported_namespace",
    },
    shared_store: true,
};
