//! The Config section's smaller kinds (E12-S12): ResourceQuota, LimitRange, HorizontalPodAutoscaler,
//! VerticalPodAutoscaler, PodDisruptionBudget, PriorityClass, RuntimeClass, Lease, the Mutating and
//! ValidatingWebhookConfigurations, ValidatingAdmissionPolicy and its binding. ConfigMaps and
//! Secrets are their own story (E12-S11).
//!
//! Each kind is a row of [`KINDS`]: its place in the sidebar and the function that turns the
//! domain view-model of its object into a [`Panel`](model::Panel). The columns come from the
//! shared catalogue (`oxikube_app::CoreColumns`, declarative JSON-pointer definitions with a few
//! computed cells); the detail is the generic one with a [`ConfigPanel`] above the Overview,
//! and the Overview keeps the metadata, conditions and events.
//!
//! | File | Holds |
//! |---|---|
//! | `model` | [`Panel`](model::Panel), `Section`, `Row`: what a panel shows, as data |
//! | `panel` | [`ConfigPanel`]: the entity that follows the detail view and caches the built panel per resource version |
//! | `draw` | the one renderer: pairs, quota bars, links, small tables, CEL code |
//! | `quotas`, `autoscaling`, `scheduling`, `admission` | the per-kind builders, each a short function over a domain view-model |
//!
//! # How a user gets here
//!
//! Sidebar, Config section, then the kind (each entry appears when the cluster's discovery serves
//! the kind and the user may list it; the VerticalPodAutoscaler only where its CRD is installed).
//! Enter on a row opens the drawer, which shows the panel above the metadata. An HPA's or VPA's
//! scale target and a binding's policy are links that open the object.
//!
//! # Performance
//!
//! Nothing here runs per frame per row: the panel is rebuilt when the object's resource version
//! changes and drawn from the cached [`Panel`](model::Panel); the table's cells are the shared
//! catalogue's.

mod admission;
mod autoscaling;
mod draw;
mod model;
mod panel;
mod quotas;
mod scheduling;

#[cfg(test)]
mod tests;

use gpui::App;
use oxikube_app::Section;
use oxikube_domain::command::CommandId;

pub use panel::ConfigPanel;

use super::builtin::{Builtin, cluster, namespaced, spec};
use super::specs::KindSpecs;
use panel::Build;

/// The kinds of this story, in sidebar order after ConfigMaps (rank 10) and Secrets (20).
const KINDS: &[(Builtin, u32, Build)] = &[
    (
        namespaced(
            "",
            "v1",
            "ResourceQuota",
            "resourcequotas",
            "Resource Quotas",
        ),
        30,
        quotas::resource_quota,
    ),
    (
        namespaced("", "v1", "LimitRange", "limitranges", "Limit Ranges"),
        40,
        quotas::limit_range,
    ),
    (
        namespaced(
            "autoscaling",
            "v2",
            "HorizontalPodAutoscaler",
            "horizontalpodautoscalers",
            "Horizontal Pod Autoscalers",
        ),
        50,
        autoscaling::hpa,
    ),
    // Optional: only where the VPA's CRDs are installed (discovery gates the entry).
    (
        namespaced(
            "autoscaling.k8s.io",
            "v1",
            "VerticalPodAutoscaler",
            "verticalpodautoscalers",
            "Vertical Pod Autoscalers",
        ),
        60,
        autoscaling::vpa,
    ),
    (
        namespaced(
            "policy",
            "v1",
            "PodDisruptionBudget",
            "poddisruptionbudgets",
            "Pod Disruption Budgets",
        ),
        70,
        scheduling::pdb,
    ),
    (
        cluster(
            "scheduling.k8s.io",
            "v1",
            "PriorityClass",
            "priorityclasses",
            "Priority Classes",
        ),
        80,
        scheduling::priority_class,
    ),
    (
        cluster(
            "node.k8s.io",
            "v1",
            "RuntimeClass",
            "runtimeclasses",
            "Runtime Classes",
        ),
        90,
        scheduling::runtime_class,
    ),
    (
        namespaced("coordination.k8s.io", "v1", "Lease", "leases", "Leases"),
        100,
        scheduling::lease,
    ),
    (
        cluster(
            "admissionregistration.k8s.io",
            "v1",
            "MutatingWebhookConfiguration",
            "mutatingwebhookconfigurations",
            "Mutating Webhooks",
        ),
        110,
        admission::webhooks,
    ),
    (
        cluster(
            "admissionregistration.k8s.io",
            "v1",
            "ValidatingWebhookConfiguration",
            "validatingwebhookconfigurations",
            "Validating Webhooks",
        ),
        120,
        admission::webhooks,
    ),
    (
        cluster(
            "admissionregistration.k8s.io",
            "v1",
            "ValidatingAdmissionPolicy",
            "validatingadmissionpolicies",
            "Validating Admission Policies",
        ),
        130,
        admission::policy,
    ),
    (
        cluster(
            "admissionregistration.k8s.io",
            "v1",
            "ValidatingAdmissionPolicyBinding",
            "validatingadmissionpolicybindings",
            "Validating Admission Policy Bindings",
        ),
        140,
        admission::binding,
    ),
];

/// Registers the kinds of this module in the Config section. No kind here has actions of its
/// own beyond "Used by" on PriorityClasses (E12-S18): they are read-only, and the generic row
/// actions (open, YAML, edit, delete) apply.
pub(super) fn init(cx: &mut App) {
    let specs = KINDS.iter().map(|(kind, rank, build)| {
        let actions: &[CommandId] = if kind.kind == "PriorityClass" {
            &[CommandId::RESOURCE_SHOW_USED_BY]
        } else {
            &[]
        };
        spec(kind, Section::CONFIG, *rank, actions).with_detail(ConfigPanel::renderer(*build))
    });
    if let Err(error) = KindSpecs::register_all(cx, specs) {
        tracing::error!(%error, "the Config kinds were not all registered");
    }
}
