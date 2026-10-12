//! Config: ConfigMaps and Secrets (E12-S11), and the rest of the section (E12-S12). The Secret
//! seed holds dummy data only.
//!
//! The cluster-scoped seeds do nothing to the objects of other tests: the webhook configurations
//! have no webhooks, the admission policy's binding selects only the test's namespace (and only
//! audits), the priority class is not the global default and no pod names the runtime class. The
//! namespaced ones stay inside the test's namespace: the quota counts ConfigMaps, the limit range
//! bounds claims, the budget and the autoscalers select or target only seeds.

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Scope;
use serde_json::{Value, json};

use super::{Api, KindCase, Seed};

/// A kind of the Config section with the generic actions only.
const fn config(
    group: &'static str,
    version: &'static str,
    kind: &'static str,
    plural: &'static str,
    scope: Scope,
    columns: &'static [&'static str],
    seed: fn(&Seed<'_>) -> Value,
) -> KindCase {
    KindCase {
        section: "config",
        plural,
        group,
        version,
        kind,
        scope,
        api: Api::BuiltIn,
        columns,
        actions: &[],
        panel: true,
        seed,
    }
}

const NAMESPACED: Scope = Scope::Namespaced;
const CLUSTER: Scope = Scope::Cluster;
const ADMISSION: &str = "admissionregistration.k8s.io";

pub(super) const CASES: &[KindCase] = &[
    KindCase {
        // k9s' used-by (E12-S18).
        actions: &[CommandId::RESOURCE_SHOW_USED_BY],
        ..config(
            "",
            "v1",
            "ConfigMap",
            "configmaps",
            NAMESPACED,
            &["Name", "Namespace", "Age", "Labels*"],
            |_| json!({"metadata": {}, "data": {"app.properties": "greeting=hello\n"}}),
        )
    },
    KindCase {
        actions: &[
            CommandId::SECRET_REVEAL,
            CommandId::SECRET_EDIT_DECODED,
            CommandId::RESOURCE_SHOW_USED_BY,
        ],
        ..config(
            "",
            "v1",
            "Secret",
            "secrets",
            NAMESPACED,
            &["Name", "Namespace", "Age", "Labels*"],
            // `dummy`, base64.
            |_| json!({"metadata": {}, "type": "Opaque", "data": {"token": "ZHVtbXk="}}),
        )
    },
    config(
        "",
        "v1",
        "ResourceQuota",
        "resourcequotas",
        NAMESPACED,
        &["Name", "Namespace", "Age", "Labels*"],
        |_| json!({"metadata": {}, "spec": {"hard": {"configmaps": "100"}}}),
    ),
    config(
        "",
        "v1",
        "LimitRange",
        "limitranges",
        NAMESPACED,
        &["Name", "Namespace", "Age", "Labels*"],
        |_| {
            json!({"metadata": {}, "spec": {"limits": [
                {"type": "PersistentVolumeClaim", "max": {"storage": "10Gi"}},
            ]}})
        },
    ),
    config(
        "autoscaling",
        "v2",
        "HorizontalPodAutoscaler",
        "horizontalpodautoscalers",
        NAMESPACED,
        &[
            "Name",
            "Namespace",
            "Reference",
            "Metrics",
            "Min Pods",
            "Max Pods",
            "Replicas",
            "Status",
            "Age",
            "Labels*",
        ],
        // The seeded Deployment has no replicas: an autoscaler never scales a workload up from 0.
        |_| {
            json!({"metadata": {}, "spec": {
                "scaleTargetRef": {"apiVersion": "apps/v1", "kind": "Deployment",
                                   "name": "seed-deployment"},
                "minReplicas": 1, "maxReplicas": 2,
                "metrics": [{"type": "Resource", "resource": {"name": "cpu",
                    "target": {"type": "Utilization", "averageUtilization": 80}}}],
            }})
        },
    ),
    KindCase {
        api: Api::Optional,
        ..config(
            "autoscaling.k8s.io",
            "v1",
            "VerticalPodAutoscaler",
            "verticalpodautoscalers",
            NAMESPACED,
            &[
                "Name",
                "Namespace",
                "Reference",
                "Mode",
                "Conditions",
                "Age",
                "Labels*",
            ],
            |_| {
                json!({"metadata": {}, "spec": {
                    "targetRef": {"apiVersion": "apps/v1", "kind": "Deployment",
                                  "name": "seed-deployment"},
                    "updatePolicy": {"updateMode": "Off"},
                }})
            },
        )
    },
    config(
        "policy",
        "v1",
        "PodDisruptionBudget",
        "poddisruptionbudgets",
        NAMESPACED,
        &[
            "Name",
            "Namespace",
            "Min Available",
            "Max Unavailable",
            "Allowed Disruptions",
            "Current Healthy",
            "Desired Healthy",
            "Age",
            "Labels*",
        ],
        |_| {
            json!({"metadata": {}, "spec": {
                "minAvailable": 1, "selector": {"matchLabels": {"app": "seed-budget"}},
            }})
        },
    ),
    KindCase {
        actions: &[CommandId::RESOURCE_SHOW_USED_BY],
        ..config(
            "scheduling.k8s.io",
            "v1",
            "PriorityClass",
            "priorityclasses",
            CLUSTER,
            &["Name", "Value", "Global Default", "Age"],
            |_| {
                json!({"metadata": {}, "value": 1, "globalDefault": false,
                       "description": "Seeded by the section matrix."})
            },
        )
    },
    config(
        "node.k8s.io",
        "v1",
        "RuntimeClass",
        "runtimeclasses",
        CLUSTER,
        &["Name", "Handler", "Age"],
        |_| json!({"metadata": {}, "handler": "oxikube-test"}),
    ),
    config(
        "coordination.k8s.io",
        "v1",
        "Lease",
        "leases",
        NAMESPACED,
        &["Name", "Namespace", "Holder", "Age", "Labels*"],
        |_| {
            json!({"metadata": {}, "spec": {"holderIdentity": "seed-holder",
                                             "leaseDurationSeconds": 15}})
        },
    ),
    config(
        ADMISSION,
        "v1",
        "MutatingWebhookConfiguration",
        "mutatingwebhookconfigurations",
        CLUSTER,
        &["Name", "Webhooks", "Age"],
        |_| json!({"metadata": {}, "webhooks": []}),
    ),
    config(
        ADMISSION,
        "v1",
        "ValidatingWebhookConfiguration",
        "validatingwebhookconfigurations",
        CLUSTER,
        &["Name", "Webhooks", "Age"],
        |_| json!({"metadata": {}, "webhooks": []}),
    ),
    config(
        ADMISSION,
        "v1",
        "ValidatingAdmissionPolicy",
        "validatingadmissionpolicies",
        CLUSTER,
        &["Name", "Validations", "Param Kind", "Age"],
        |_| {
            json!({"metadata": {}, "spec": {
                "failurePolicy": "Ignore",
                "matchConstraints": {"resourceRules": [{"apiGroups": [""], "apiVersions": ["v1"],
                    "operations": ["CREATE"], "resources": ["configmaps"]}]},
                "validations": [{"expression": "true"}],
            }})
        },
    ),
    config(
        ADMISSION,
        "v1",
        "ValidatingAdmissionPolicyBinding",
        "validatingadmissionpolicybindings",
        CLUSTER,
        &["Name", "Policy", "Actions", "Age"],
        policy_binding,
    ),
];

/// Binds the seeded policy (`<stem>-validatingadmissionpolicy`) to the test's namespace alone,
/// auditing only.
fn policy_binding(seed: &Seed<'_>) -> Value {
    json!({"metadata": {}, "spec": {
        "policyName": format!("{}-validatingadmissionpolicy", seed.stem),
        "validationActions": ["Audit"],
        "matchResources": {"namespaceSelector": {"matchLabels": {
            "kubernetes.io/metadata.name": seed.namespace,
        }}},
    }})
}
