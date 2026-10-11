//! kube-prometheus and kube-prometheus-stack (the Prometheus Operator).
//!
//! Assumptions, from the stack's manifests and rules:
//!
//! * cAdvisor series (scraped from the kubelet by its ServiceMonitor) carry `namespace`, `pod`
//!   and `container`; the pause container is `container="POD"` on older runtimes.
//! * kube-state-metrics v2 (`kube_pod_container_resource_requests{resource,unit}`), as shipped
//!   since the stack moved to ksm 2.0.
//! * node-exporter series are labelled `instance`; whether that is the node name or `ip:9100`
//!   differs between the jsonnet and the Helm chart, so nodes are matched through
//!   `node_uname_info{nodename}`, which every node-exporter exposes.
//! * Ingress controller metrics are scraped through a ServiceMonitor without `honorLabels`, so
//!   the metric's own `namespace` label is renamed `exported_namespace`.

use oxikube_domain::MetricsProviderKind;

use super::profile::{IngressSource, Profile};

pub(super) const PROFILE: Profile = Profile {
    kind: MetricsProviderKind::KubePrometheus,
    ingress: IngressSource::Nginx {
        namespace_label: "exported_namespace",
    },
    shared_store: false,
};
