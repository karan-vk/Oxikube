//! OpenShift's monitoring stack (the Thanos querier in `openshift-monitoring`).
//!
//! Assumptions:
//!
//! * cAdvisor, kube-state-metrics v2 and node-exporter series as in kube-prometheus; the
//!   platform stack scrapes them with the same labels.
//! * There is no ingress-nginx: Ingresses are served by the HAProxy router, whose
//!   `haproxy_backend_bytes_in_total` / `_out_total` carry `route` and `exported_namespace`.
//!   The Route created for an Ingress is named `<ingress>-<suffix>`, so an Ingress is matched
//!   by `route=~"<ingress>|<ingress>-[a-z0-9]+"`.
//! * The querier enforces namespaces through its tenancy port; this catalogue targets the
//!   cluster-wide port the detector finds.

use oxikube_domain::MetricsProviderKind;

use super::profile::{IngressSource, Profile};

pub(super) const PROFILE: Profile = Profile {
    kind: MetricsProviderKind::OpenShift,
    ingress: IngressSource::HaproxyRouter,
    shared_store: false,
};
