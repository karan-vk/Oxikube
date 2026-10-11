//! The metrics stack Lens installs (`lens-metrics/prometheus`).
//!
//! Assumptions:
//!
//! * The same cAdvisor, kube-state-metrics v2 and node-exporter series as kube-prometheus (Lens'
//!   own queries use `pod`, `container`, `namespace` and `resource="cpu"`).
//! * Targets are found by annotations (`prometheus.io/scrape`) and the stack relabels the
//!   target's namespace to `kubernetes_namespace`, so a metric's own `namespace` label is not
//!   renamed: ingress-nginx metrics keep `namespace`.

use oxikube_domain::MetricsProviderKind;

use super::profile::{IngressSource, Profile};

pub(super) const PROFILE: Profile = Profile {
    kind: MetricsProviderKind::Lens,
    ingress: IngressSource::Nginx {
        namespace_label: "namespace",
    },
    shared_store: false,
};
