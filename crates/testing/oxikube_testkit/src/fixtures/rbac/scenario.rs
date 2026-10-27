//! [`RbacScenario`]: the objects of the RBAC kind suites (E17-S10), as plain JSON, so the kind
//! tests apply them to a cluster and the offline tests index them without one (the same objects
//! both ways, so the offline test catches a fixture that drifts).
//!
//! Two namespaces, `home` and `other`; in `home` a ServiceAccount [`ACCOUNT`] that may `get`,
//! `list` and `watch` pods there (a Role and its RoleBinding) and nothing else of its own. The
//! suites then grant it the built-in aggregated ClusterRole `view` in `other` with
//! [`RbacScenario::view_binding`], the binding the binding dialog builds.

use oxikube_domain::rbac::Subject;
use serde_json::{Value, json};

/// The restricted ServiceAccount, in the `home` namespace.
pub const ACCOUNT: &str = "limited";
/// The Role (and its RoleBinding) that lets [`ACCOUNT`] read pods in `home`.
const POD_READER: &str = "limited-pods";
/// The RoleBinding in `other` that grants [`ACCOUNT`] the ClusterRole `view`.
pub const VIEW_BINDING: &str = "limited-view";
/// The verbs the Role grants on pods.
pub const POD_VERBS: [&str; 3] = ["get", "list", "watch"];

const RBAC_GROUP: &str = "rbac.authorization.k8s.io";

/// The scenario's two namespaces; every object it makes lives in one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RbacScenario {
    /// Where the account lives and may read pods.
    pub home: String,
    /// Where it may do nothing until [`view_binding`](Self::view_binding) is applied.
    pub other: String,
}

impl RbacScenario {
    /// The scenario over the namespaces `home` and `other`.
    pub fn new(home: &str, other: &str) -> Self {
        Self {
            home: home.to_owned(),
            other: other.to_owned(),
        }
    }

    /// The account as a binding subject.
    pub fn account(&self) -> Subject {
        Subject::service_account(&self.home, ACCOUNT)
    }

    /// The user name the API server gives the account:
    /// `system:serviceaccount:<home>:limited`.
    pub fn username(&self) -> String {
        format!("system:serviceaccount:{}:{ACCOUNT}", self.home)
    }

    /// The namespaces, the account, its Role and its RoleBinding, in the order to apply them.
    pub fn objects(&self) -> Vec<Value> {
        let namespace = |name: &str| json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": name}});
        vec![
            namespace(&self.home),
            namespace(&self.other),
            json!({
                "apiVersion": "v1", "kind": "ServiceAccount",
                "metadata": {"name": ACCOUNT, "namespace": self.home},
            }),
            json!({
                "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
                "metadata": {"name": POD_READER, "namespace": self.home},
                "rules": [{"apiGroups": [""], "resources": ["pods"], "verbs": POD_VERBS}],
            }),
            json!({
                "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
                "metadata": {"name": POD_READER, "namespace": self.home},
                "roleRef": {"apiGroup": RBAC_GROUP, "kind": "Role", "name": POD_READER},
                "subjects": [{"kind": "ServiceAccount", "name": ACCOUNT, "namespace": self.home}],
            }),
        ]
    }

    /// The objects as one `List`, as `kubectl apply -f -` takes them.
    pub fn list(&self) -> Value {
        json!({"apiVersion": "v1", "kind": "List", "items": self.objects()})
    }

    /// A RoleBinding in `other` granting the account the built-in aggregated ClusterRole
    /// `view`: what the binding dialog builds for "ClusterRole view, ServiceAccount
    /// `<home>/limited`, in `other`".
    pub fn view_binding(&self) -> Value {
        json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
            "metadata": {"name": VIEW_BINDING, "namespace": self.other},
            "roleRef": {"apiGroup": RBAC_GROUP, "kind": "ClusterRole", "name": "view"},
            "subjects": [{"kind": "ServiceAccount", "name": ACCOUNT, "namespace": self.home}],
        })
    }
}
