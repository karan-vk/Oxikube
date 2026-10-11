//! Discovery sets for the kind registry and sidebar tests (E12-S01): what a whole API server
//! serves, as `discover()` returns it.
//!
//! | Set | Holds |
//! |---|---|
//! | [`core`] | a stock cluster: [`core_kinds`](crate::kinds::core_kinds) plus the other built-in kinds every current server serves |
//! | [`core_with_gateway_api`] | [`core`] plus the Gateway API CRDs (an optional group the sidebar shows only when served) |
//! | [`core_with_crds`] | [`core`] plus cert-manager's CRDs (the Custom Resources section) |
//! | [`crds_only`] | cert-manager's CRDs and nothing built in (a server whose discovery lists only custom groups) |

use oxikube_domain::kinds::ResourceKind;

use crate::kinds::{KindSpec, cert_manager_kinds, core_kinds};

/// The built-in kinds [`core_kinds`] leaves out that every current server serves.
fn more_built_ins() -> Vec<ResourceKind> {
    let k = |group: &str, version: &str, kind_name: &str, plural: &str| {
        KindSpec::new(group, version, kind_name, plural).singular(&kind_name.to_ascii_lowercase())
    };
    vec![
        k("", "v1", "ReplicationController", "replicationcontrollers")
            .short("rc")
            .build(),
        k("", "v1", "ResourceQuota", "resourcequotas")
            .short("quota")
            .build(),
        k("", "v1", "LimitRange", "limitranges")
            .short("limits")
            .build(),
        k("discovery.k8s.io", "v1", "EndpointSlice", "endpointslices").build(),
        k("networking.k8s.io", "v1", "IngressClass", "ingressclasses")
            .cluster_scoped()
            .build(),
        k("coordination.k8s.io", "v1", "Lease", "leases").build(),
        k(
            "scheduling.k8s.io",
            "v1",
            "PriorityClass",
            "priorityclasses",
        )
        .short("pc")
        .cluster_scoped()
        .build(),
        k("node.k8s.io", "v1", "RuntimeClass", "runtimeclasses")
            .cluster_scoped()
            .build(),
        k(
            "admissionregistration.k8s.io",
            "v1",
            "MutatingWebhookConfiguration",
            "mutatingwebhookconfigurations",
        )
        .cluster_scoped()
        .build(),
        k(
            "admissionregistration.k8s.io",
            "v1",
            "ValidatingWebhookConfiguration",
            "validatingwebhookconfigurations",
        )
        .cluster_scoped()
        .build(),
        k(
            "admissionregistration.k8s.io",
            "v1",
            "ValidatingAdmissionPolicy",
            "validatingadmissionpolicies",
        )
        .cluster_scoped()
        .build(),
        k(
            "admissionregistration.k8s.io",
            "v1",
            "ValidatingAdmissionPolicyBinding",
            "validatingadmissionpolicybindings",
        )
        .cluster_scoped()
        .build(),
    ]
}

/// The Gateway API's standard kinds (`gateway.networking.k8s.io`).
pub fn gateway_api_kinds() -> Vec<ResourceKind> {
    let k = |kind_name: &str, plural: &str| {
        KindSpec::new("gateway.networking.k8s.io", "v1", kind_name, plural)
            .singular(&kind_name.to_ascii_lowercase())
    };
    vec![
        k("GatewayClass", "gatewayclasses").cluster_scoped().build(),
        k("Gateway", "gateways").build(),
        k("HTTPRoute", "httproutes").build(),
        k("GRPCRoute", "grpcroutes").build(),
    ]
}

/// A stock cluster: every built-in kind, no optional group, no CRD.
pub fn core() -> Vec<ResourceKind> {
    let mut kinds = core_kinds();
    kinds.extend(more_built_ins());
    kinds
}

/// [`core`] with the Gateway API installed.
pub fn core_with_gateway_api() -> Vec<ResourceKind> {
    let mut kinds = core();
    kinds.extend(gateway_api_kinds());
    kinds
}

/// [`core`] with cert-manager installed.
pub fn core_with_crds() -> Vec<ResourceKind> {
    let mut kinds = core();
    kinds.extend(cert_manager_kinds());
    kinds
}

/// cert-manager's CRDs alone.
pub fn crds_only() -> Vec<ResourceKind> {
    cert_manager_kinds()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn the_sets_are_unique_per_group_and_plural() {
        for set in [
            core(),
            core_with_gateway_api(),
            core_with_crds(),
            crds_only(),
        ] {
            let mut seen = BTreeSet::new();
            for kind in &set {
                assert!(
                    seen.insert((kind.gvk.group.clone(), kind.plural.clone())),
                    "{kind} twice"
                );
            }
        }
    }

    #[test]
    fn only_the_gateway_set_serves_the_gateway_api() {
        let gateway = |kinds: &[ResourceKind]| {
            kinds
                .iter()
                .any(|k| &*k.gvk.group == "gateway.networking.k8s.io")
        };
        assert!(!gateway(&core()));
        assert!(!gateway(&core_with_crds()));
        assert!(gateway(&core_with_gateway_api()));
    }
}
