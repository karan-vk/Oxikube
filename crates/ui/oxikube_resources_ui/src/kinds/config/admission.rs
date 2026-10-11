//! Mutating and ValidatingWebhookConfiguration, ValidatingAdmissionPolicy and its binding.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::view::{
    AdmissionBindingSummary, AdmissionPolicySummary, WebhookConfigSummary, WebhookRule,
};

use super::model::{LinkTo, Panel, Row};

/// The rules of a webhook or policy as a table: operations, resources, scope.
fn rules_table(rules: &[WebhookRule]) -> Vec<Row> {
    if rules.is_empty() {
        return Vec::new();
    }
    let mut rows = vec![Row::head([
        "Operations",
        "API groups",
        "Resources",
        "Scope",
    ])];
    // An empty group name is the core group.
    let join = |list: &[Arc<str>], empty: &str| {
        if list.is_empty() {
            empty.to_owned()
        } else {
            list.iter()
                .map(|s| if s.is_empty() { "core" } else { &**s })
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    rows.extend(rules.iter().map(|rule| {
        Row::cells([
            join(&rule.operations, "-"),
            join(&rule.api_groups, "core"),
            join(&rule.resources, "-"),
            rule.scope.as_deref().unwrap_or("*").to_owned(),
        ])
    }));
    rows
}

/// A webhook configuration: one section per webhook with its target, policies, selectors and
/// rules. The `caBundle` shows as present or absent, never its content.
pub(super) fn webhooks(res: &Resource) -> Option<Panel> {
    let config = WebhookConfigSummary::from_resource(res).ok()?;
    let mut panel = Panel::new();
    for webhook in &config.webhooks {
        let mut rows = vec![
            Row::pair("Target", webhook.target.text()),
            Row::pair("CA bundle", webhook.ca_bundle.as_str()),
            Row::pair("Failure policy", webhook.failure_policy.to_string()),
        ];
        rows.extend(Row::opt("Side effects", webhook.side_effects.as_deref()));
        rows.push(Row::pair("Match policy", webhook.match_policy.to_string()));
        rows.push(Row::pair(
            "Timeout",
            format!("{}s", webhook.timeout_seconds),
        ));
        rows.extend(Row::opt(
            "Reinvocation policy",
            webhook.reinvocation_policy.as_deref(),
        ));
        if !webhook.review_versions.is_empty() {
            let versions: Vec<&str> = webhook.review_versions.iter().map(|v| &**v).collect();
            rows.push(Row::pair("Review versions", versions.join(", ")));
        }
        rows.extend(Row::opt(
            "Namespace selector",
            webhook.namespace_selector.as_deref(),
        ));
        rows.extend(Row::opt(
            "Object selector",
            webhook.object_selector.as_deref(),
        ));
        if webhook.match_conditions > 0 {
            rows.push(Row::pair(
                "Match conditions",
                webhook.match_conditions.to_string(),
            ));
        }
        rows.extend(rules_table(&webhook.rules));
        panel = panel.section(format!("Webhook {}", webhook.name), rows);
    }
    if panel.is_empty() {
        panel = panel.section("Webhooks", vec![Row::Note("No webhooks.".to_owned())]);
    }
    Some(panel)
}

/// A ValidatingAdmissionPolicy: failure policy, parameter kind, match rules and the CEL
/// validations.
pub(super) fn policy(res: &Resource) -> Option<Panel> {
    let policy = AdmissionPolicySummary::from_resource(res).ok()?;
    let mut summary = vec![Row::pair(
        "Failure policy",
        policy.failure_policy.to_string(),
    )];
    summary.extend(Row::opt(
        "Param kind",
        policy.param_kind.as_ref().map(|p| {
            if p.api_version.is_empty() {
                p.kind.to_string()
            } else {
                format!("{} ({})", p.kind, p.api_version)
            }
        }),
    ));
    for (label, n) in [
        ("Match conditions", policy.match_conditions),
        ("Variables", policy.variables),
        ("Audit annotations", policy.audit_annotations),
    ] {
        if n > 0 {
            summary.push(Row::pair(label, n.to_string()));
        }
    }
    let mut validations = Vec::new();
    for (ix, v) in policy.validations.iter().enumerate() {
        validations.push(Row::Heading(format!("Validation {}", ix + 1)));
        validations.push(Row::Code(v.expression.to_string()));
        let message = v
            .message
            .as_deref()
            .or(v.message_expression.as_deref())
            .map(str::to_owned);
        let text = match (message, v.reason.as_deref()) {
            (Some(m), Some(r)) => Some(format!("{m} ({r})")),
            (Some(m), None) => Some(m),
            (None, Some(r)) => Some(r.to_owned()),
            (None, None) => None,
        };
        validations.extend(text.map(Row::Note));
    }
    Some(
        Panel::new()
            .section("Policy", summary)
            .section("Match rules", rules_table(&policy.rules))
            .section("Validations", validations),
    )
}

/// A ValidatingAdmissionPolicyBinding: the policy it binds (a link), what it does on a failure,
/// its parameters and what it applies to.
pub(super) fn binding(res: &Resource) -> Option<Panel> {
    let binding = AdmissionBindingSummary::from_resource(res).ok()?;
    let mut rows = Vec::new();
    if !binding.policy_name.is_empty() {
        rows.push(Row::Link {
            label: "Policy".to_owned(),
            text: binding.policy_name.to_string(),
            to: LinkTo {
                gvk: Gvk::new(
                    "admissionregistration.k8s.io",
                    "v1",
                    "ValidatingAdmissionPolicy",
                ),
                name: binding.policy_name.clone(),
                scope: Scope::Cluster,
            },
        });
    }
    rows.push(Row::pair("Actions", binding.actions_text()));
    if let Some(param) = &binding.param_ref {
        let mut parts = Vec::new();
        parts.extend(param.name.as_deref().map(str::to_owned));
        parts.extend(param.selector.as_ref().map(|s| format!("selector {s}")));
        parts.extend(param.namespace.as_deref().map(|n| format!("in {n}")));
        rows.push(Row::pair("Params", parts.join(" ")));
        rows.extend(Row::opt(
            "If params are missing",
            param.not_found_action.as_deref(),
        ));
    }
    rows.extend(Row::opt(
        "Namespace selector",
        binding.namespace_selector.as_deref(),
    ));
    rows.extend(Row::opt(
        "Object selector",
        binding.object_selector.as_deref(),
    ));
    Some(Panel::new().section("Binding", rows))
}
