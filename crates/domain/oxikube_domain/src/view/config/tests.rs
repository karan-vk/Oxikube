//! Tests of the Config view-models over inline manifests.

use serde_json::{Value, json};

use super::*;
use crate::resource::Resource;
use crate::view::ViewError;

fn res(api_version: &str, kind: &str, rest: Value) -> Resource {
    let mut obj = json!({
        "apiVersion": api_version,
        "kind": kind,
        "metadata": {"name": "x", "namespace": "demo", "creationTimestamp": "2026-01-01T00:00:00Z"},
    });
    for (k, v) in rest.as_object().cloned().unwrap_or_default() {
        obj[k] = v;
    }
    Resource::from_json(obj).expect("a resource")
}

fn quota(hard: Value, used: Value) -> QuotaSummary {
    QuotaSummary::from_resource(&res(
        "v1",
        "ResourceQuota",
        json!({"status": {"hard": hard, "used": used}}),
    ))
    .expect("a quota")
}

fn line<'a>(q: &'a QuotaSummary, name: &str) -> &'a QuotaLine {
    q.lines
        .iter()
        .find(|l| &*l.resource == name)
        .unwrap_or_else(|| panic!("no line {name}"))
}

#[test]
fn quota_ratios_cover_quantities_counts_and_extended_resources() {
    let q = quota(
        json!({"pods": "10", "requests.cpu": "4", "limits.memory": "8Gi",
               "count/deployments.apps": "5", "requests.nvidia.com/gpu": "2"}),
        json!({"pods": "9", "requests.cpu": "1500m", "limits.memory": "2Gi",
               "count/deployments.apps": "5", "requests.nvidia.com/gpu": "0"}),
    );
    assert_eq!(line(&q, "pods").percent(), Some(90));
    assert_eq!(line(&q, "pods").level(), QuotaLevel::Warn);
    assert_eq!(line(&q, "requests.cpu").percent(), Some(37));
    assert_eq!(line(&q, "requests.cpu").text(), "1500m / 4");
    assert_eq!(line(&q, "limits.memory").percent(), Some(25));
    let deployments = line(&q, "count/deployments.apps");
    assert!(deployments.is_count());
    assert_eq!(deployments.level(), QuotaLevel::Full);
    assert_eq!(line(&q, "requests.nvidia.com/gpu").percent(), Some(0));
    assert!(!line(&q, "requests.cpu").is_count());
    let classed = quota(
        json!({"gold.storageclass.storage.k8s.io/persistentvolumeclaims": "4"}),
        json!({}),
    );
    assert!(
        classed.lines[0].is_count(),
        "claims of one storage class are a count"
    );
    assert!(line(&q, "pods").is_count());
}

#[test]
fn quota_lines_are_sorted_and_cope_with_gaps() {
    let q = quota(
        json!({"requests.cpu": "0", "pods": "bogus"}),
        json!({"requests.cpu": "1", "services": "2"}),
    );
    let names: Vec<&str> = q.lines.iter().map(|l| &*l.resource).collect();
    assert_eq!(names, ["pods", "requests.cpu", "services"]);
    // A zero limit that is exceeded reads as over the limit, not as a missing ratio.
    assert_eq!(line(&q, "requests.cpu").level(), QuotaLevel::Full);
    assert_eq!(line(&q, "requests.cpu").percent(), Some(100_000));
    // Unparseable or one-sided lines have no ratio.
    assert_eq!(line(&q, "pods").ratio, None);
    assert_eq!(line(&q, "services").ratio, None);
    assert_eq!(line(&q, "services").text(), "2 / -");
}

#[test]
fn quota_falls_back_to_spec_hard_and_reads_scopes() {
    let q = QuotaSummary::from_resource(&res(
        "v1",
        "ResourceQuota",
        json!({"spec": {"hard": {"pods": "3"}, "scopes": ["BestEffort"],
                         "scopeSelector": {"matchExpressions": [
                             {"scopeName": "PriorityClass", "operator": "In", "values": ["high", "low"]}]}}}),
    ))
    .unwrap();
    assert_eq!(q.lines.len(), 1);
    assert_eq!(q.lines[0].hard.as_deref(), Some("3"));
    assert_eq!(q.lines[0].used, None);
    assert_eq!(q.scopes.len(), 1);
    assert_eq!(q.scope_selector, ["PriorityClass In (high,low)"]);
}

#[test]
fn limit_ranges_merge_the_maps_per_resource() {
    let lr = LimitRangeSummary::from_resource(&res(
        "v1",
        "LimitRange",
        json!({"spec": {"limits": [
            {"type": "Container", "min": {"cpu": "50m"}, "max": {"cpu": "2", "memory": "1Gi"},
             "default": {"cpu": "500m"}, "defaultRequest": {"cpu": "100m"},
             "maxLimitRequestRatio": {"cpu": "10"}},
            {"max": {"storage": "50Gi"}}
        ]}}),
    ))
    .unwrap();
    assert_eq!(lr.sections.len(), 2);
    let container = &lr.sections[0];
    assert_eq!(&*container.kind, "Container");
    let cpu = &container.rows[0];
    assert_eq!(&*cpu.resource, "cpu");
    assert_eq!(cpu.min.as_deref(), Some("50m"));
    assert_eq!(cpu.max.as_deref(), Some("2"));
    assert_eq!(cpu.default.as_deref(), Some("500m"));
    assert_eq!(cpu.default_request.as_deref(), Some("100m"));
    assert_eq!(cpu.max_ratio.as_deref(), Some("10"));
    assert_eq!(&*container.rows[1].resource, "memory");
    assert_eq!(container.rows[1].default, None);
    // A missing type reads as Container, the API's common case.
    assert_eq!(&*lr.sections[1].kind, "Container");
}

fn hpa(spec: Value, status: Value) -> HpaSummary {
    HpaSummary::from_resource(&res(
        "autoscaling/v2",
        "HorizontalPodAutoscaler",
        json!({"spec": spec, "status": status}),
    ))
    .unwrap()
}

#[test]
fn hpa_metrics_text_covers_resource_pods_object_and_external_metrics() {
    let h = hpa(
        json!({
        "scaleTargetRef": {"apiVersion": "apps/v1", "kind": "Deployment", "name": "web"},
        "minReplicas": 2, "maxReplicas": 10,
        "metrics": [
            {"type": "Resource", "resource": {"name": "cpu", "target": {"type": "Utilization", "averageUtilization": 80}}},
            {"type": "Pods", "pods": {"metric": {"name": "tps"}, "target": {"type": "AverageValue", "averageValue": "500m"}}},
            {"type": "Object", "object": {"metric": {"name": "rps"}, "describedObject": {"kind": "Ingress", "name": "web"}, "target": {"type": "Value", "value": "2k"}}},
            {"type": "External", "external": {"metric": {"name": "queue_depth"}, "target": {"type": "Value", "value": "30"}}},
            {"type": "ContainerResource", "containerResource": {"name": "memory", "container": "app", "target": {"type": "AverageValue", "averageValue": "512Mi"}}}
        ]}),
        json!({
        "currentReplicas": 3, "desiredReplicas": 4,
        "currentMetrics": [
            {"type": "Resource", "resource": {"name": "cpu", "current": {"averageUtilization": 42, "averageValue": "42m"}}},
            {"type": "Pods", "pods": {"metric": {"name": "tps"}, "current": {"averageValue": "300m"}}},
            {"type": "Object", "object": {"metric": {"name": "rps"}, "describedObject": {"kind": "Ingress", "name": "web"}, "current": {"value": "1k"}}},
            {"type": "External", "external": {"metric": {"name": "queue_depth"}, "current": {"value": "12"}}}
        ]}),
    );
    assert_eq!(
        h.metrics_text().as_deref(),
        Some("42%/80%, 300m/500m, 1k/2k, 12/30, <unknown>/512Mi")
    );
    assert_eq!(
        h.reference.as_ref().map(TargetRef::display).as_deref(),
        Some("Deployment/web")
    );
    assert_eq!(h.reference.as_ref().unwrap().gvk().api_version(), "apps/v1");
    assert_eq!((h.min_replicas, h.max_replicas), (2, Some(10)));
    assert_eq!((h.current_replicas, h.desired_replicas), (Some(3), Some(4)));
    assert_eq!(h.metrics[2].title(), "Object rps on Ingress/web");
    assert_eq!(h.metrics[4].title(), "ContainerResource memory on app");
}

#[test]
fn hpa_v1_cpu_fields_read_as_one_metric() {
    let h = HpaSummary::from_resource(&res(
        "autoscaling/v1",
        "HorizontalPodAutoscaler",
        json!({"spec": {"scaleTargetRef": {"kind": "Deployment", "name": "web"}, "targetCPUUtilizationPercentage": 70, "maxReplicas": 5},
               "status": {"currentCPUUtilizationPercentage": 35, "currentReplicas": 2}}),
    ))
    .unwrap();
    assert_eq!(h.metrics_text().as_deref(), Some("35%/70%"));
    assert_eq!(h.min_replicas, 1, "the API default");
    let none = hpa(json!({"maxReplicas": 3}), json!({}));
    assert_eq!(none.metrics_text(), None);
}

#[test]
fn hpa_state_reads_the_conditions() {
    let state = |conditions: Value| hpa(json!({}), json!({"conditions": conditions})).state;
    assert_eq!(state(json!([])), HpaState::Unknown);
    assert_eq!(
        state(
            json!([{"type": "AbleToScale", "status": "True"}, {"type": "ScalingActive", "status": "True"}])
        ),
        HpaState::Active
    );
    assert_eq!(
        state(json!([{"type": "ScalingActive", "status": "True"},
                     {"type": "ScalingLimited", "status": "True", "reason": "TooManyReplicas"}])),
        HpaState::Limited("TooManyReplicas".into())
    );
    let inactive = state(
        json!([{"type": "ScalingActive", "status": "False", "reason": "FailedGetResourceMetric"}]),
    );
    assert_eq!(inactive.text(), "FailedGetResourceMetric");
    assert!(matches!(inactive, HpaState::Inactive(_)));
}

#[test]
fn pdb_allowed_disruptions_come_from_the_controller_else_the_counts() {
    let pdb = |spec: Value, status: Value| {
        PdbSummary::from_resource(&res(
            "policy/v1",
            "PodDisruptionBudget",
            json!({"spec": spec, "status": status}),
        ))
        .unwrap()
    };
    let reported = pdb(
        json!({"minAvailable": "50%", "selector": {"matchLabels": {"app": "web"}}}),
        json!({"disruptionsAllowed": 1, "currentHealthy": 3, "desiredHealthy": 2, "expectedPods": 3,
               "disruptedPods": {"web-1": "2026-01-01T00:00:00Z"}}),
    );
    assert_eq!(reported.min_available.as_deref(), Some("50%"));
    assert_eq!(reported.max_unavailable, None);
    assert_eq!(reported.allowed_disruptions(), 1);
    assert_eq!(reported.selector.as_deref(), Some("app=web"));
    assert_eq!(reported.disrupted_pods, 1);
    assert!(!reported.is_blocking());
    let derived = pdb(
        json!({"maxUnavailable": 1}),
        json!({"currentHealthy": 5, "desiredHealthy": 3}),
    );
    assert_eq!(
        derived.max_unavailable.as_deref(),
        Some("1"),
        "numbers read as text"
    );
    assert_eq!(derived.allowed_disruptions(), 2);
    let short = pdb(json!({}), json!({"currentHealthy": 1, "desiredHealthy": 3}));
    assert_eq!(short.allowed_disruptions(), 0);
    assert!(short.is_blocking());
}

#[test]
fn lease_holder_and_times() {
    let lease = LeaseSummary::from_resource(&res(
        "coordination.k8s.io/v1",
        "Lease",
        json!({"spec": {"holderIdentity": "node-a_1f2e", "leaseDurationSeconds": 15,
                         "acquireTime": "2026-01-01T00:00:00.000000Z",
                         "renewTime": "2026-01-02T03:04:00.000000Z", "leaseTransitions": 2}}),
    ))
    .unwrap();
    assert_eq!(lease.holder_text(), "node-a_1f2e");
    assert_eq!(lease.transitions, Some(2));
    assert_eq!(lease.duration_seconds, Some(15));
    let free =
        LeaseSummary::from_resource(&res("coordination.k8s.io/v1", "Lease", json!({"spec": {}})))
            .unwrap();
    assert_eq!(free.holder_text(), "");
}

#[test]
fn priority_and_runtime_classes() {
    let p = PriorityClassSummary::from_resource(&res(
        "scheduling.k8s.io/v1",
        "PriorityClass",
        json!({"value": 1000000, "globalDefault": true, "description": "d"}),
    ))
    .unwrap();
    assert_eq!((p.value, p.global_default), (1_000_000, true));
    assert_eq!(&*p.preemption_policy, "PreemptLowerPriority");
    let r = RuntimeClassSummary::from_resource(&res(
        "node.k8s.io/v1",
        "RuntimeClass",
        json!({"handler": "runsc", "overhead": {"podFixed": {"cpu": "250m"}},
               "scheduling": {"nodeSelector": {"sandbox": "gvisor"},
                               "tolerations": [{"key": "sandbox", "operator": "Exists", "effect": "NoSchedule"},
                                               {"key": "a", "operator": "Equal", "value": "b"}]}}),
    ))
    .unwrap();
    assert_eq!(&*r.handler, "runsc");
    assert_eq!(r.overhead, [("cpu".into(), "250m".into())]);
    assert_eq!(r.node_selector, [("sandbox".into(), "gvisor".into())]);
    assert_eq!(r.tolerations, ["sandbox:NoSchedule", "a=b"]);
}

#[test]
fn webhook_configurations_never_keep_the_ca_bundle() {
    const BLOB: &str = "RFVNTVktQ0EtQlVORExF";
    let cfg = WebhookConfigSummary::from_resource(&res(
        "admissionregistration.k8s.io/v1",
        "MutatingWebhookConfiguration",
        json!({"webhooks": [
            {"name": "a.example.com", "failurePolicy": "Ignore", "sideEffects": "None", "timeoutSeconds": 5,
             "reinvocationPolicy": "IfNeeded", "admissionReviewVersions": ["v1"],
             "clientConfig": {"service": {"namespace": "ns", "name": "svc", "path": "/m", "port": 8443}, "caBundle": BLOB},
             "namespaceSelector": {"matchLabels": {"inject": "on"}},
             "objectSelector": {"matchExpressions": [{"key": "skip", "operator": "DoesNotExist"}]},
             "rules": [{"operations": ["CREATE", "UPDATE"], "apiGroups": [""], "apiVersions": ["v1"], "resources": ["pods"], "scope": "Namespaced"}],
             "matchConditions": [{"name": "c", "expression": "true"}]},
            {"name": "b.example.com", "clientConfig": {"url": "https://b.example.com/x"}}
        ]}),
    ))
    .unwrap();
    assert_eq!(cfg.kind, WebhookKind::Mutating);
    assert_eq!(cfg.webhooks.len(), 2);
    let a = &cfg.webhooks[0];
    assert_eq!(a.ca_bundle, CaBundle::Present);
    assert_eq!(a.target.text(), "ns/svc:8443/m");
    assert_eq!((&*a.failure_policy, a.timeout_seconds), ("Ignore", 5));
    assert_eq!(a.namespace_selector.as_deref(), Some("inject=on"));
    assert_eq!(a.object_selector.as_deref(), Some("!skip"));
    assert_eq!(a.rules[0].operations, ["CREATE".into(), "UPDATE".into()]);
    assert_eq!(
        a.rules[0].api_groups,
        ["".into()],
        "the core group is the empty name"
    );
    assert_eq!(a.match_conditions, 1);
    let b = &cfg.webhooks[1];
    assert_eq!(b.ca_bundle, CaBundle::Absent);
    assert_eq!(
        b.target,
        WebhookTarget::Url("https://b.example.com/x".into())
    );
    assert_eq!(
        (&*b.failure_policy, b.timeout_seconds, &*b.match_policy),
        ("Fail", 10, "Equivalent")
    );
    assert!(!format!("{cfg:?}").contains(BLOB), "the blob is not kept");
    assert_eq!(CaBundle::Absent.as_str(), "Absent");
}

#[test]
fn admission_policies_and_bindings() {
    let policy = AdmissionPolicySummary::from_resource(&res(
        "admissionregistration.k8s.io/v1",
        "ValidatingAdmissionPolicy",
        json!({"spec": {"paramKind": {"apiVersion": "rules.example.com/v1", "kind": "Limit"},
                         "matchConstraints": {"resourceRules": [{"apiGroups": ["apps"], "apiVersions": ["v1"], "operations": ["CREATE"], "resources": ["deployments"]}]},
                         "validations": [{"expression": "a < b", "message": "m", "reason": "Invalid"}, {"expression": "true"}],
                         "variables": [{"name": "v", "expression": "1"}]},
               "status": {"conditions": [{"type": "TypeChecked", "status": "True"}]}}),
    ))
    .unwrap();
    assert_eq!(policy.validations.len(), 2);
    assert_eq!(&*policy.failure_policy, "Fail");
    assert_eq!(
        policy
            .param_kind
            .as_ref()
            .map(TargetRef::display)
            .as_deref(),
        Some("Limit")
    );
    assert_eq!(
        (
            policy.variables,
            policy.rules.len(),
            policy.conditions.len()
        ),
        (1, 1, 1)
    );
    let binding = AdmissionBindingSummary::from_resource(&res(
        "admissionregistration.k8s.io/v1",
        "ValidatingAdmissionPolicyBinding",
        json!({"spec": {"policyName": "p", "validationActions": ["Deny", "Audit"],
                         "paramRef": {"name": "prod", "parameterNotFoundAction": "Deny"},
                         "matchResources": {"namespaceSelector": {"matchLabels": {"env": "prod"}}}}}),
    ))
    .unwrap();
    assert_eq!(&*binding.policy_name, "p");
    assert_eq!(binding.actions_text(), "Deny,Audit");
    assert_eq!(
        binding.param_ref.as_ref().and_then(|p| p.name.as_deref()),
        Some("prod")
    );
    assert_eq!(binding.namespace_selector.as_deref(), Some("env=prod"));
}

#[test]
fn vpa_defaults_the_mode_and_reads_recommendations() {
    let vpa = VpaSummary::from_resource(&res(
        "autoscaling.k8s.io/v1",
        "VerticalPodAutoscaler",
        json!({"spec": {"targetRef": {"apiVersion": "apps/v1", "kind": "Deployment", "name": "web"},
                         "resourcePolicy": {"containerPolicies": [{"containerName": "*", "mode": "Off", "maxAllowed": {"cpu": "2"}}]}},
               "status": {"conditions": [{"type": "RecommendationProvided", "status": "True"}, {"type": "LowConfidence", "status": "False"}],
                          "recommendation": {"containerRecommendations": [{"containerName": "web", "target": {"cpu": "120m", "memory": "200Mi"}, "lowerBound": {"cpu": "25m"}}]}}}),
    ))
    .unwrap();
    assert_eq!(&*vpa.mode, "Auto");
    assert_eq!(vpa.conditions_text(), "RecommendationProvided");
    assert_eq!(vpa.recommendations[0].target.len(), 2);
    assert_eq!(vpa.recommendations[0].lower, [("cpu".into(), "25m".into())]);
    assert_eq!(vpa.policies[0].mode.as_deref(), Some("Off"));
    assert_eq!(vpa.target.unwrap().display(), "Deployment/web");
}

#[test]
fn selectors_print_like_kubectl() {
    let text = |v: Value| {
        let r = res("v1", "ConfigMap", json!({"data": v}));
        selector_text(r.json().get("data").unwrap())
    };
    assert_eq!(text(json!({})).as_deref(), Some("<all>"));
    assert_eq!(text(json!("nope")), None);
    assert_eq!(
        text(json!({"matchLabels": {"b": "2", "a": "1"},
                    "matchExpressions": [{"key": "t", "operator": "In", "values": ["x", "y"]},
                                         {"key": "u", "operator": "NotIn", "values": ["z"]},
                                         {"key": "v", "operator": "Exists"}]}))
        .as_deref(),
        Some("a=1,b=2,t in (x,y),u notin (z),v")
    );
}

#[test]
fn the_wrong_kind_is_refused() {
    let pod = res("v1", "Pod", json!({}));
    assert!(matches!(
        QuotaSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
    assert!(HpaSummary::from_resource(&pod).is_err());
    assert!(PdbSummary::from_resource(&pod).is_err());
    assert!(LeaseSummary::from_resource(&pod).is_err());
    assert!(WebhookConfigSummary::from_resource(&pod).is_err());
    assert!(VpaSummary::from_resource(&pod).is_err());
    assert!(RuntimeClassSummary::from_resource(&pod).is_err());
    assert!(PriorityClassSummary::from_resource(&pod).is_err());
    assert!(AdmissionPolicySummary::from_resource(&pod).is_err());
    assert!(AdmissionBindingSummary::from_resource(&pod).is_err());
    assert!(LimitRangeSummary::from_resource(&pod).is_err());
}

#[test]
fn an_empty_ca_bundle_is_absent() {
    let cfg = WebhookConfigSummary::from_resource(&res(
        "admissionregistration.k8s.io/v1",
        "ValidatingWebhookConfiguration",
        json!({"webhooks": [{"name": "w", "clientConfig": {"service": {"name": "s", "namespace": "n"}, "caBundle": ""}}]}),
    ))
    .unwrap();
    assert_eq!(
        cfg.webhooks[0].ca_bundle,
        CaBundle::Absent,
        "cert-manager has not injected it yet"
    );
}

#[test]
fn metrics_with_the_same_name_are_told_apart_by_their_selector() {
    let h = hpa(
        json!({"metrics": [
            {"type": "External", "external": {"metric": {"name": "queue", "selector": {"matchLabels": {"q": "a"}}}, "target": {"type": "Value", "value": "10"}}},
            {"type": "External", "external": {"metric": {"name": "queue", "selector": {"matchLabels": {"q": "b"}}}, "target": {"type": "Value", "value": "20"}}}]}),
        json!({"currentMetrics": [
            {"type": "External", "external": {"metric": {"name": "queue", "selector": {"matchLabels": {"q": "b"}}}, "current": {"value": "7"}}},
            {"type": "External", "external": {"metric": {"name": "queue", "selector": {"matchLabels": {"q": "a"}}}, "current": {"value": "3"}}}]}),
    );
    assert_eq!(h.metrics_text().as_deref(), Some("3/10, 7/20"));
    assert_eq!(h.metrics[1].title(), "External queue [q=b]");
}

#[test]
fn a_target_scaled_to_zero_disables_the_autoscaler_without_a_fault() {
    let state = hpa(
        json!({}),
        json!({"conditions": [{"type": "ScalingActive", "status": "False", "reason": "ScalingDisabled"}]}),
    )
    .state;
    assert_eq!(state, HpaState::Disabled);
    assert_eq!(state.text(), "ScalingDisabled");
}
