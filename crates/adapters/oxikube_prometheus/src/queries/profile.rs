//! [`Profile`]: how one stack differs from the common conventions.

/// Where an Ingress's traffic is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IngressSource {
    /// ingress-nginx's `nginx_ingress_controller_*` metrics, which carry an `ingress` label and
    /// the Ingress's namespace under `namespace_label`.
    Nginx { namespace_label: &'static str },
    /// OpenShift's HAProxy router (`haproxy_backend_bytes_*`): the Routes an Ingress creates
    /// are named after it, so the Ingress is found by route name.
    HaproxyRouter,
}

/// What differs between stacks. Everything else (cAdvisor `pod`/`container`/`namespace`
/// labels, kube-state-metrics v2 names, node-exporter metrics) is shared and lives in the
/// builders; each stack's module documents the conventions it was written against.
#[derive(Debug, Clone, Copy)]
pub(super) struct Profile {
    pub ingress: IngressSource,
    /// The store usually holds several clusters, so an unscoped query mixes them: callers add
    /// an external-label matcher (`MetricRequest::with_label`).
    pub shared_store: bool,
}
