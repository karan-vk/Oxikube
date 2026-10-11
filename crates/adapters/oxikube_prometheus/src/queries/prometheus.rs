//! A plain Prometheus-compatible API: a stand-alone Prometheus (the community Helm chart),
//! Thanos or Cortex, or a URL the user configured without naming a stack.
//!
//! The conventions are those of kube-prometheus (see that module), the most common layout: the
//! kubelet's cAdvisor `namespace`/`pod`/`container` labels, kube-state-metrics v2, node-exporter
//! matched by `node_uname_info`, and ingress-nginx under `exported_namespace` (a scrape job that
//! does not set `honor_labels` renames the metric's own `namespace`).

use oxikube_domain::MetricsProviderKind;

use super::profile::{IngressSource, Profile};

pub(super) const PROFILE: Profile = Profile {
    kind: MetricsProviderKind::Prometheus,
    ingress: IngressSource::Nginx {
        namespace_label: "exported_namespace",
    },
    shared_store: false,
};
