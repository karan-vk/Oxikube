//! The Config section's kinds (E12-S12): their columns, and the check that the declarative
//! JSON-pointer columns read the same text as the domain view-models' hand-written formatters.

use oxikube_domain::view::{HpaSummary, LeaseSummary, PdbSummary};
use oxikube_testkit::fixtures as fx;

use super::{check, text};
use crate::columns::{ColumnProvider, CoreColumns, Tone};
use oxikube_domain::Capabilities;

#[test]
fn hpa_columns_match_the_list_in_the_plan() {
    let columns = CoreColumns::new().columns(&fx::hpa().kind, Capabilities::all());
    let titles: Vec<&str> = columns
        .iter()
        .filter(|c| !c.wide)
        .map(|c| &*c.title)
        .collect();
    assert_eq!(
        titles,
        [
            "Name",
            "Namespace",
            "Reference",
            "Metrics",
            "Min Pods",
            "Max Pods",
            "Replicas",
            "Status",
            "Age"
        ]
    );
}

#[test]
fn hpa_status_reads_the_conditions_with_a_tone() {
    let hpa = |conditions: serde_json::Value| {
        let mut json = fx::json("autoscaling/hpa.json");
        json["status"]["conditions"] = conditions;
        oxikube_domain::Resource::from_json(json).unwrap()
    };
    let provider = CoreColumns::new();
    let status = |res: &oxikube_domain::Resource| {
        let cell = provider.resource_cell(res, &"status".into(), super::now());
        (cell.display().to_owned(), cell.tone())
    };
    assert_eq!(status(&fx::hpa()), ("Unknown".to_owned(), Tone::Neutral));
    let active = hpa(serde_json::json!([{"type": "ScalingActive", "status": "True"}]));
    assert_eq!(status(&active), ("Active".to_owned(), Tone::Ok));
    let limited = hpa(serde_json::json!([
        {"type": "ScalingActive", "status": "True"},
        {"type": "ScalingLimited", "status": "True", "reason": "TooManyReplicas"}
    ]));
    assert_eq!(status(&limited), ("TooManyReplicas".to_owned(), Tone::Warn));
    let broken = hpa(serde_json::json!([
        {"type": "ScalingActive", "status": "False", "reason": "FailedGetResourceMetric"}
    ]));
    assert_eq!(
        status(&broken),
        ("FailedGetResourceMetric".to_owned(), Tone::Error)
    );
}

#[test]
fn the_vpa_has_mode_conditions_and_a_reference() {
    check(
        &fx::vpa(),
        &[
            ("name", "web"),
            ("namespace", "demo"),
            ("reference", "Deployment/web"),
            ("mode", "Auto"),
            ("conditions", "RecommendationProvided"),
            ("age", "27h"),
        ],
    );
    let mut json = fx::json("config/vpa.json");
    json["spec"]["updatePolicy"] = serde_json::json!({"updateMode": "Off"});
    json["status"] = serde_json::json!({});
    let off = oxikube_domain::Resource::from_json(json).unwrap();
    check(&off, &[("mode", "Off"), ("conditions", "")]);
}

#[test]
fn the_lease_shows_its_holder() {
    check(
        &fx::lease(),
        &[
            ("name", "kube-controller-manager"),
            ("namespace", "kube-system"),
            ("holder", "oxikube-control-plane_7c1e2d9a"),
            ("age", "27h"),
        ],
    );
}

#[test]
fn the_remaining_kinds_have_the_columns_the_plan_lists() {
    check(
        &fx::priority_class(),
        &[("value", "1000000"), ("global-default", "false")],
    );
    check(&fx::runtime_class(), &[("handler", "runsc")]);
    check(&fx::mutating_webhook(), &[("webhooks", "2")]);
    check(&fx::validating_webhook(), &[("webhooks", "1")]);
    check(&fx::admission_policy(), &[("validations", "2")]);
    check(
        &fx::admission_policy_binding(),
        &[("policy", "replica-limit"), ("actions", "Deny,Audit")],
    );
    check(
        &fx::resource_quota(),
        &[("name", "compute"), ("age", "27h")],
    );
    check(&fx::limit_range(), &[("name", "defaults"), ("age", "27h")]);
    check(
        &fx::pdb(),
        &[
            ("min-available", "2"),
            ("max-unavailable", ""),
            ("allowed-disruptions", "1"),
            ("current-healthy", "3"),
            ("desired-healthy", "2"),
        ],
    );
}

/// The columns are declarative (a JSON pointer and a type); the view-models are the hand-written
/// formatters. On the fixtures they read the same text, so a table cell and a panel row agree.
#[test]
fn declarative_columns_read_the_same_text_as_the_view_models() {
    let hpa = fx::hpa();
    let summary = HpaSummary::from_resource(&hpa).unwrap();
    assert_eq!(text(&hpa, "targets"), summary.metrics_text().unwrap());
    assert_eq!(text(&hpa, "min-pods"), summary.min_replicas.to_string());
    assert_eq!(
        text(&hpa, "max-pods"),
        summary.max_replicas.unwrap().to_string()
    );
    assert_eq!(
        text(&hpa, "replicas"),
        summary.current_replicas.unwrap().to_string()
    );

    let pdb = fx::pdb();
    let summary = PdbSummary::from_resource(&pdb).unwrap();
    assert_eq!(
        text(&pdb, "min-available"),
        summary.min_available.as_deref().unwrap()
    );
    assert_eq!(
        text(&pdb, "allowed-disruptions"),
        summary.allowed_disruptions().to_string()
    );
    assert_eq!(
        text(&pdb, "current-healthy"),
        summary.current_healthy.to_string()
    );
    assert_eq!(
        text(&pdb, "desired-healthy"),
        summary.desired_healthy.to_string()
    );

    let lease = fx::lease();
    let summary = LeaseSummary::from_resource(&lease).unwrap();
    assert_eq!(
        text(&lease, "holder"),
        summary.holder.as_deref().unwrap_or("")
    );
}

#[test]
fn a_pdb_without_a_reported_allowance_shows_the_computed_one_like_the_panel() {
    let mut json = fx::json("config/pdb.json");
    json["status"]
        .as_object_mut()
        .unwrap()
        .remove("disruptionsAllowed");
    let pdb = oxikube_domain::Resource::from_json(json).unwrap();
    // 3 healthy against 2 desired.
    check(&pdb, &[("allowed-disruptions", "1")]);
    assert_eq!(
        text(&pdb, "allowed-disruptions"),
        PdbSummary::from_resource(&pdb)
            .unwrap()
            .allowed_disruptions()
            .to_string()
    );
}

#[test]
fn a_scaled_to_zero_hpa_is_neutral_not_an_error() {
    let mut json = fx::json("autoscaling/hpa.json");
    json["status"]["conditions"] = serde_json::json!([
        {"type": "ScalingActive", "status": "False", "reason": "ScalingDisabled"}
    ]);
    let hpa = oxikube_domain::Resource::from_json(json).unwrap();
    let cell = CoreColumns::new().resource_cell(&hpa, &"status".into(), super::now());
    assert_eq!(
        (cell.display(), cell.tone()),
        ("ScalingDisabled", Tone::Neutral)
    );
}
