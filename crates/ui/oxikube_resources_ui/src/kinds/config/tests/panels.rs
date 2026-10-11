//! The panel of every Config kind, built from its fixture.

use oxikube_app::columns::Tone;
use oxikube_domain::view::QuotaLevel;
use oxikube_testkit::fixtures as fx;

use super::super::model::{Panel, Row};
use super::super::{admission, autoscaling, quotas, scheduling};

fn rows<'a>(panel: &'a Panel, section: &str) -> &'a [Row] {
    &panel
        .sections
        .iter()
        .find(|s| s.title.eq_ignore_ascii_case(section))
        .unwrap_or_else(|| panic!("no section {section} in {:?}", panel.sections))
        .rows
}

fn pair<'a>(panel: &'a Panel, label: &str) -> &'a str {
    match panel.find(|r| matches!(r, Row::Pair { label: l, .. } if l == label)) {
        Some(Row::Pair { value, .. }) => value,
        _ => panic!("no row {label} in {}", panel.all_text()),
    }
}

#[test]
fn a_quota_draws_a_bar_per_resource_and_groups_the_counts() {
    let panel = quotas::resource_quota(&fx::resource_quota()).unwrap();
    let resources: Vec<&str> = rows(&panel, "Resources")
        .iter()
        .map(|r| match r {
            Row::Bar { label, .. } => label.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        resources,
        ["limits.memory", "requests.cpu", "requests.nvidia.com/gpu"]
    );
    let counts: Vec<&str> = rows(&panel, "Object counts")
        .iter()
        .map(|r| match r {
            Row::Bar { label, .. } => label.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(counts, ["count/deployments.apps", "pods"]);
    assert_eq!(
        rows(&panel, "Object counts")[1],
        Row::Bar {
            label: "pods".into(),
            text: "9 / 10".into(),
            percent: Some(90),
            level: QuotaLevel::Warn,
        }
    );
    assert_eq!(
        rows(&panel, "Resources")[1],
        Row::Bar {
            label: "requests.cpu".into(),
            text: "1500m / 4".into(),
            percent: Some(37),
            level: QuotaLevel::Ok,
        }
    );
    assert_eq!(pair(&panel, "Scopes"), "NotTerminating");
}

#[test]
fn a_quota_without_limits_says_so() {
    let empty = oxikube_domain::Resource::from_json(serde_json::json!({
        "apiVersion": "v1", "kind": "ResourceQuota",
        "metadata": {"name": "none", "namespace": "demo"}
    }))
    .unwrap();
    let panel = quotas::resource_quota(&empty).unwrap();
    assert!(panel.all_text().contains("No limits set."));
}

#[test]
fn a_limit_range_is_a_table_per_type() {
    let panel = quotas::limit_range(&fx::limit_range()).unwrap();
    let container = rows(&panel, "Container limits");
    assert_eq!(
        container[0],
        Row::head([
            "Resource",
            "Min",
            "Max",
            "Default",
            "Default request",
            "Max ratio"
        ])
    );
    assert_eq!(
        container[1],
        Row::cells(["cpu", "50m", "2", "500m", "100m", "10"])
    );
    assert_eq!(
        container[2],
        Row::cells(["memory", "-", "1Gi", "256Mi", "64Mi", "-"])
    );
    let pvc = rows(&panel, "PersistentVolumeClaim limits");
    assert_eq!(
        pvc[1],
        Row::cells(["storage", "1Gi", "50Gi", "-", "-", "-"])
    );
}

#[test]
fn an_hpa_links_its_scale_target_and_tabulates_metrics() {
    let panel = autoscaling::hpa(&fx::hpa()).unwrap();
    let scaling = rows(&panel, "Scaling");
    let Row::Link { text, to, .. } = &scaling[0] else {
        panic!("the scale target comes first, as a link: {scaling:?}");
    };
    assert_eq!(text, "Deployment/web");
    assert_eq!(to.gvk.api_version(), "apps/v1");
    assert_eq!(&*to.name, "web");
    assert_eq!(pair(&panel, "Min replicas"), "2");
    assert_eq!(pair(&panel, "Max replicas"), "10");
    assert_eq!(pair(&panel, "Current replicas"), "3");
    assert_eq!(pair(&panel, "Status"), "Unknown");
    let metrics = rows(&panel, "Metrics");
    assert_eq!(metrics[0], Row::head(["Metric", "Current", "Target"]));
    assert_eq!(metrics[1], Row::cells(["Resource cpu", "42%", "80%"]));
    assert_eq!(
        metrics[2],
        Row::cells(["Resource memory", "<unknown>", "512Mi"])
    );
}

#[test]
fn an_hpa_status_carries_its_tone() {
    let mut json = fx::json("autoscaling/hpa.json");
    json["status"]["conditions"] = serde_json::json!([
        {"type": "ScalingActive", "status": "False", "reason": "FailedGetResourceMetric"}
    ]);
    let hpa = oxikube_domain::Resource::from_json(json).unwrap();
    let panel = autoscaling::hpa(&hpa).unwrap();
    assert_eq!(
        panel.find(|r| matches!(r, Row::Pair { label, .. } if label == "Status")),
        Some(&Row::Pair {
            label: "Status".into(),
            value: "FailedGetResourceMetric".into(),
            tone: Tone::Error,
        })
    );
}

#[test]
fn a_vpa_shows_mode_recommendations_and_policies() {
    let panel = autoscaling::vpa(&fx::vpa()).unwrap();
    assert_eq!(pair(&panel, "Update mode"), "Auto");
    assert_eq!(pair(&panel, "Min replicas"), "2");
    assert!(matches!(rows(&panel, "Autoscaler")[0], Row::Link { .. }));
    let recommendations = rows(&panel, "Recommendations");
    assert_eq!(
        recommendations[1],
        Row::cells([
            "web",
            "cpu 25m, memory 64Mi",
            "cpu 120m, memory 200Mi",
            "cpu 1, memory 1Gi"
        ])
    );
    let policies = rows(&panel, "Container policies");
    assert_eq!(
        policies[1],
        Row::cells(["*", "-", "cpu 50m", "cpu 2, memory 2Gi", "cpu, memory"])
    );
}

#[test]
fn a_pdb_shows_the_budget_and_the_allowed_disruptions() {
    let panel = scheduling::pdb(&fx::pdb()).unwrap();
    assert_eq!(pair(&panel, "Min available"), "2");
    assert_eq!(pair(&panel, "Selector"), "app=web");
    assert_eq!(pair(&panel, "Allowed disruptions"), "1");
    assert_eq!(pair(&panel, "Current healthy"), "3");
    assert_eq!(pair(&panel, "Desired healthy"), "2");
    assert_eq!(pair(&panel, "Unhealthy pod eviction"), "IfHealthyBudget");
    // Zero allowed disruptions is flagged: a drain would wait on it.
    let mut json = fx::json("config/pdb.json");
    json["status"]["disruptionsAllowed"] = serde_json::json!(0);
    let blocked = scheduling::pdb(&oxikube_domain::Resource::from_json(json).unwrap()).unwrap();
    assert!(matches!(
        blocked.find(|r| matches!(r, Row::Pair { label, .. } if label == "Allowed disruptions")),
        Some(Row::Pair {
            tone: Tone::Warn,
            ..
        })
    ));
}

#[test]
fn a_lease_shows_its_holder_and_times() {
    let panel = scheduling::lease(&fx::lease()).unwrap();
    assert_eq!(pair(&panel, "Holder"), "oxikube-control-plane_7c1e2d9a");
    assert_eq!(pair(&panel, "Lease duration"), "15s");
    assert_eq!(pair(&panel, "Transitions"), "2");
    assert!(pair(&panel, "Renewed").starts_with("2026-01-02T03:04:00"));
}

#[test]
fn priority_and_runtime_classes() {
    let priority = scheduling::priority_class(&fx::priority_class()).unwrap();
    assert_eq!(pair(&priority, "Value"), "1000000");
    assert_eq!(pair(&priority, "Global default"), "No");
    assert_eq!(pair(&priority, "Preemption policy"), "PreemptLowerPriority");
    assert_eq!(pair(&priority, "Description"), "Customer-facing services.");
    let runtime = scheduling::runtime_class(&fx::runtime_class()).unwrap();
    assert_eq!(pair(&runtime, "Handler"), "runsc");
    assert_eq!(pair(&runtime, "Pod overhead"), "cpu 250m, memory 120Mi");
    assert_eq!(pair(&runtime, "Node selector"), "sandbox=gvisor");
    assert_eq!(pair(&runtime, "Tolerations"), "sandbox:NoSchedule");
}

#[test]
fn a_webhook_configuration_lists_its_webhooks_and_hides_the_ca_bundle() {
    let res = fx::mutating_webhook();
    let panel = admission::webhooks(&res).unwrap();
    let titles: Vec<&str> = panel.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Webhook inject.example.com", "Webhook defaults.example.com"]
    );
    let first = rows(&panel, "Webhook inject.example.com");
    let value = |label: &str| match first
        .iter()
        .find(|r| matches!(r, Row::Pair { label: l, .. } if l == label))
    {
        Some(Row::Pair { value, .. }) => value.as_str(),
        _ => panic!("no {label}"),
    };
    assert_eq!(value("Target"), "injector/injector:443/mutate");
    assert_eq!(value("CA bundle"), "Present");
    assert_eq!(value("Failure policy"), "Ignore");
    assert_eq!(value("Side effects"), "None");
    assert_eq!(value("Timeout"), "10s");
    assert_eq!(value("Reinvocation policy"), "IfNeeded");
    assert_eq!(value("Namespace selector"), "injection=enabled");
    assert!(first.contains(&Row::cells(["CREATE", "core", "pods", "Namespaced"])));
    let second = rows(&panel, "Webhook defaults.example.com");
    assert!(second.contains(&Row::pair("Target", "https://defaults.example.com/mutate")));
    assert!(second.contains(&Row::pair("CA bundle", "Absent")));
    // The blob never reaches the panel, in any row.
    let blob = res
        .get_str("/webhooks/0/clientConfig/caBundle")
        .expect("the fixture has a caBundle");
    assert!(!panel.all_text().contains(blob));
    assert!(!format!("{panel:?}").contains(blob));
    let validating = admission::webhooks(&fx::validating_webhook()).unwrap();
    assert_eq!(validating.sections.len(), 1);
    assert!(
        rows(&validating, "Webhook check.example.com")
            .contains(&Row::pair("Object selector", "!skip-check"))
    );
}

#[test]
fn a_policy_shows_its_cel_validations_and_a_binding_links_the_policy() {
    let policy = admission::policy(&fx::admission_policy()).unwrap();
    assert_eq!(pair(&policy, "Failure policy"), "Fail");
    assert_eq!(
        pair(&policy, "Param kind"),
        "ReplicaLimit (rules.example.com/v1)"
    );
    let validations = rows(&policy, "Validations");
    assert!(validations.contains(&Row::Code(
        "object.spec.replicas <= params.maxReplicas".into()
    )));
    assert!(validations.contains(&Row::Note("too many replicas (Invalid)".into())));
    assert!(validations.contains(&Row::Code("object.spec.replicas >= 1".into())));
    assert!(rows(&policy, "Match rules").contains(&Row::cells([
        "CREATE, UPDATE",
        "apps",
        "deployments",
        "*"
    ])));

    let binding = admission::binding(&fx::admission_policy_binding()).unwrap();
    let Row::Link { text, to, .. } = &rows(&binding, "Binding")[0] else {
        panic!("the policy is a link");
    };
    assert_eq!(text, "replica-limit");
    assert_eq!(&*to.gvk.kind, "ValidatingAdmissionPolicy");
    assert_eq!(to.scope, oxikube_domain::ids::Scope::Cluster);
    assert_eq!(pair(&binding, "Actions"), "Deny,Audit");
    assert_eq!(pair(&binding, "Params"), "prod-limits");
    assert_eq!(pair(&binding, "If params are missing"), "Deny");
    assert_eq!(pair(&binding, "Namespace selector"), "env=prod");
}

#[test]
fn a_builder_refuses_another_kind() {
    let pod = oxikube_testkit::fixtures::pod_running();
    for build in [
        quotas::resource_quota,
        quotas::limit_range,
        autoscaling::hpa,
        autoscaling::vpa,
        scheduling::pdb,
        scheduling::lease,
        scheduling::priority_class,
        scheduling::runtime_class,
        admission::webhooks,
        admission::policy,
        admission::binding,
    ] {
        assert!(build(&pod).is_none());
    }
}
