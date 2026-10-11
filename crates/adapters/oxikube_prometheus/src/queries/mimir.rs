//! Grafana Mimir (the query frontend, `/prometheus` prefix).
//!
//! Assumptions:
//!
//! * Prometheus or Grafana Agent remote-writes the usual Kubernetes series (cAdvisor,
//!   kube-state-metrics v2, node-exporter) with the scraper's labels, so the conventions are
//!   kube-prometheus' (`exported_namespace` for ingress-nginx).
//! * A Mimir tenant usually holds several clusters, told apart by an external label such as
//!   `cluster`: callers add it with `MetricRequest::with_label`, otherwise the query aggregates
//!   them all. The tenant header (`X-Scope-OrgID`) belongs to the endpoint, not the query.

use super::profile::{IngressSource, Profile};

pub(super) const PROFILE: Profile = Profile {
    ingress: IngressSource::Nginx {
        namespace_label: "exported_namespace",
    },
    shared_store: true,
};
