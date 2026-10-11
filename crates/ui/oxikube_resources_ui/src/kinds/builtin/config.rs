//! Config: ConfigMaps, Secrets, quotas and limits, autoscalers, disruption budgets, priority and
//! runtime classes, leases, admission webhooks and policies.

use gpui::App;
use oxikube_app::Section;

use super::{Builtin, cluster, generic, namespaced, register};

const CONFIG: &[Builtin] = &[
    namespaced("", "v1", "ConfigMap", "configmaps", "ConfigMaps"),
    namespaced("", "v1", "Secret", "secrets", "Secrets"),
    namespaced(
        "",
        "v1",
        "ResourceQuota",
        "resourcequotas",
        "Resource Quotas",
    ),
    namespaced("", "v1", "LimitRange", "limitranges", "Limit Ranges"),
    namespaced(
        "autoscaling",
        "v2",
        "HorizontalPodAutoscaler",
        "horizontalpodautoscalers",
        "Horizontal Pod Autoscalers",
    ),
    // Optional: only where the VPA's CRDs are installed.
    namespaced(
        "autoscaling.k8s.io",
        "v1",
        "VerticalPodAutoscaler",
        "verticalpodautoscalers",
        "Vertical Pod Autoscalers",
    ),
    namespaced(
        "policy",
        "v1",
        "PodDisruptionBudget",
        "poddisruptionbudgets",
        "Pod Disruption Budgets",
    ),
    cluster(
        "scheduling.k8s.io",
        "v1",
        "PriorityClass",
        "priorityclasses",
        "Priority Classes",
    ),
    cluster(
        "node.k8s.io",
        "v1",
        "RuntimeClass",
        "runtimeclasses",
        "Runtime Classes",
    ),
    namespaced("coordination.k8s.io", "v1", "Lease", "leases", "Leases"),
    cluster(
        "admissionregistration.k8s.io",
        "v1",
        "MutatingWebhookConfiguration",
        "mutatingwebhookconfigurations",
        "Mutating Webhooks",
    ),
    cluster(
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingWebhookConfiguration",
        "validatingwebhookconfigurations",
        "Validating Webhooks",
    ),
    cluster(
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingAdmissionPolicy",
        "validatingadmissionpolicies",
        "Validating Admission Policies",
    ),
    cluster(
        "admissionregistration.k8s.io",
        "v1",
        "ValidatingAdmissionPolicyBinding",
        "validatingadmissionpolicybindings",
        "Validating Admission Policy Bindings",
    ),
];

/// Registers the Config section's kinds.
pub fn init(cx: &mut App) {
    register(cx, Section::CONFIG, CONFIG, generic);
}
