//! PodDisruptionBudget, Lease, PriorityClass and RuntimeClass.

use oxikube_app::columns::Tone;
use oxikube_domain::Resource;
use oxikube_domain::view::{LeaseSummary, PdbSummary, PriorityClassSummary, RuntimeClassSummary};

use super::model::{Panel, Row};

/// A PDB: the budget it enforces, and how many pods may leave right now.
pub(super) fn pdb(res: &Resource) -> Option<Panel> {
    let pdb = PdbSummary::from_resource(res).ok()?;
    let mut budget = Vec::new();
    budget.extend(Row::opt("Min available", pdb.min_available.as_deref()));
    budget.extend(Row::opt("Max unavailable", pdb.max_unavailable.as_deref()));
    budget.extend(Row::opt("Selector", pdb.selector.as_deref()));
    budget.extend(Row::opt(
        "Unhealthy pod eviction",
        pdb.unhealthy_policy.as_deref(),
    ));
    let allowed = pdb.allowed_disruptions();
    let tone = if pdb.is_blocking() {
        Tone::Warn
    } else {
        Tone::Ok
    };
    let status = vec![
        Row::pair_toned("Allowed disruptions", allowed.to_string(), tone),
        Row::pair("Current healthy", pdb.current_healthy.to_string()),
        Row::pair("Desired healthy", pdb.desired_healthy.to_string()),
        Row::pair("Expected pods", pdb.expected_pods.to_string()),
        Row::pair("Disrupted pods", pdb.disrupted_pods.to_string()),
    ];
    Some(
        Panel::new()
            .section("Budget", budget)
            .section("Status", status),
    )
}

/// A Lease: its holder and when it was acquired and last renewed.
pub(super) fn lease(res: &Resource) -> Option<Panel> {
    let lease = LeaseSummary::from_resource(res).ok()?;
    let mut rows = vec![Row::pair(
        "Holder",
        lease.holder.as_deref().unwrap_or("<none>"),
    )];
    rows.extend(Row::opt(
        "Lease duration",
        lease.duration_seconds.map(|s| format!("{s}s")),
    ));
    rows.extend(Row::opt("Acquired", lease.acquired.map(|t| t.to_string())));
    rows.extend(Row::opt("Renewed", lease.renewed.map(|t| t.to_string())));
    rows.extend(Row::opt(
        "Transitions",
        lease.transitions.map(|n| n.to_string()),
    ));
    rows.extend(Row::opt(
        "Preferred holder",
        lease.preferred_holder.as_deref(),
    ));
    Some(Panel::new().section("Lease", rows))
}

/// A PriorityClass: its value, whether it is the default and how it preempts.
pub(super) fn priority_class(res: &Resource) -> Option<Panel> {
    let class = PriorityClassSummary::from_resource(res).ok()?;
    let mut rows = vec![
        Row::pair("Value", class.value.to_string()),
        Row::pair(
            "Global default",
            if class.global_default { "Yes" } else { "No" },
        ),
        Row::pair("Preemption policy", class.preemption_policy.to_string()),
    ];
    rows.extend(Row::opt("Description", class.description.as_deref()));
    Some(Panel::new().section("Priority", rows))
}

/// A RuntimeClass: its handler, pod overhead and where its pods may run.
pub(super) fn runtime_class(res: &Resource) -> Option<Panel> {
    let class = RuntimeClassSummary::from_resource(res).ok()?;
    let pairs = |list: &[(std::sync::Arc<str>, std::sync::Arc<str>)], sep: &str| {
        list.iter()
            .map(|(k, v)| format!("{k}{sep}{v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut rows = vec![Row::pair("Handler", class.handler.to_string())];
    if !class.overhead.is_empty() {
        rows.push(Row::pair("Pod overhead", pairs(&class.overhead, " ")));
    }
    let mut scheduling = Vec::new();
    if !class.node_selector.is_empty() {
        scheduling.push(Row::pair("Node selector", pairs(&class.node_selector, "=")));
    }
    if !class.tolerations.is_empty() {
        scheduling.push(Row::pair("Tolerations", class.tolerations.join(", ")));
    }
    Some(
        Panel::new()
            .section("Runtime", rows)
            .section("Scheduling", scheduling),
    )
}
