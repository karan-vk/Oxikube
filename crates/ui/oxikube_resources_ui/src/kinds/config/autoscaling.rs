//! HorizontalPodAutoscaler and VerticalPodAutoscaler.

use std::sync::Arc;

use oxikube_app::columns::hpa_state_tone;
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::view::{HpaSummary, TargetRef, VpaSummary};

use super::model::{LinkTo, Panel, Row, join_pairs};

/// The scale target as a link: the workload the autoscaler scales, in the autoscaler's namespace.
/// An `apiVersion`-less reference to a built-in workload kind gets its usual group; any other
/// `apiVersion`-less kind cannot be located without discovery, so it is shown as text.
fn target_link(label: &str, target: &TargetRef) -> Row {
    let gvk = if target.api_version.is_empty() {
        match &*target.kind {
            "Deployment" | "StatefulSet" | "ReplicaSet" | "DaemonSet" => {
                Gvk::new("apps", "v1", &*target.kind)
            }
            "ReplicationController" => Gvk::new("", "v1", "ReplicationController"),
            _ => return Row::pair(label, target.display()),
        }
    } else {
        target.gvk()
    };
    Row::Link {
        label: label.to_owned(),
        text: target.display(),
        to: LinkTo {
            gvk,
            name: target.name.clone(),
            scope: Scope::Namespaced,
        },
    }
}

fn count(n: Option<i64>) -> Option<String> {
    n.map(|n| n.to_string())
}

/// An HPA: the scale target, replica bounds and counts, and a table of metrics against targets.
pub(super) fn hpa(res: &Resource) -> Option<Panel> {
    let hpa = HpaSummary::from_resource(res).ok()?;
    let mut scaling = Vec::new();
    scaling.extend(
        hpa.reference
            .as_ref()
            .map(|t| target_link("Scale target", t)),
    );
    scaling.push(Row::pair_toned(
        "Status",
        hpa.state.text(),
        hpa_state_tone(&hpa.state),
    ));
    scaling.push(Row::pair("Min replicas", hpa.min_replicas.to_string()));
    scaling.extend(Row::opt("Max replicas", count(hpa.max_replicas)));
    scaling.extend(Row::opt("Current replicas", count(hpa.current_replicas)));
    scaling.extend(Row::opt("Desired replicas", count(hpa.desired_replicas)));
    scaling.extend(Row::opt(
        "Last scale",
        hpa.last_scale.map(|t| t.to_string()),
    ));

    let mut metrics = Vec::new();
    if !hpa.metrics.is_empty() {
        metrics.push(Row::head(["Metric", "Current", "Target"]));
        metrics.extend(hpa.metrics.iter().map(|m| {
            Row::cells([
                m.title(),
                m.current_text().to_owned(),
                m.target_text().to_owned(),
            ])
        }));
    }
    Some(
        Panel::new()
            .section("Scaling", scaling)
            .section("Metrics", metrics),
    )
}

/// A resource list as `cpu 100m, memory 1Gi`; `-` when empty.
fn resources(list: &[(Arc<str>, Arc<str>)]) -> String {
    if list.is_empty() {
        "-".to_owned()
    } else {
        join_pairs(list, " ")
    }
}

/// A VPA: the target, the update mode, the recommendation per container and the container
/// policies.
pub(super) fn vpa(res: &Resource) -> Option<Panel> {
    let vpa = VpaSummary::from_resource(res).ok()?;
    let mut summary = Vec::new();
    summary.extend(vpa.target.as_ref().map(|t| target_link("Target", t)));
    summary.push(Row::pair("Update mode", vpa.mode.to_string()));
    summary.extend(Row::opt(
        "Min replicas",
        vpa.min_replicas.map(|n| n.to_string()),
    ));
    let mut recommendations = Vec::new();
    if !vpa.recommendations.is_empty() {
        recommendations.push(Row::head([
            "Container",
            "Lower bound",
            "Target",
            "Upper bound",
        ]));
        recommendations.extend(vpa.recommendations.iter().map(|r| {
            Row::cells([
                r.container.to_string(),
                resources(&r.lower),
                resources(&r.target),
                resources(&r.upper),
            ])
        }));
    }
    let mut policies = Vec::new();
    if !vpa.policies.is_empty() {
        policies.push(Row::head([
            "Container",
            "Mode",
            "Min allowed",
            "Max allowed",
            "Controls",
        ]));
        policies.extend(vpa.policies.iter().map(|p| {
            let controlled: Vec<&str> = p.controlled.iter().map(|c| &**c).collect();
            Row::cells([
                p.container.to_string(),
                p.mode.as_deref().unwrap_or("-").to_owned(),
                resources(&p.min_allowed),
                resources(&p.max_allowed),
                if controlled.is_empty() {
                    "-".to_owned()
                } else {
                    controlled.join(", ")
                },
            ])
        }));
    }
    Some(
        Panel::new()
            .section("Autoscaler", summary)
            .section("Recommendations", recommendations)
            .section("Container policies", policies),
    )
}
