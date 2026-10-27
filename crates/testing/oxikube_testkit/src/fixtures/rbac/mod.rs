//! Recorded RBAC fixtures (E17-S01) under `fixtures/rbac/`: what a v1.37 kind cluster held and
//! answered for the objects of `fixture.yaml`, re-recorded with `record.sh`.
//!
//! - [`cluster`]: the Roles, ClusterRoles (built-in `view`, `edit`, `admin`, `cluster-admin`
//!   with their aggregation sources, the `system:*` roles every authenticated user holds, and the
//!   fixture's own) and bindings the reviews depend on.
//! - [`reviews`]: per subject and namespace, who the API server took the subject for
//!   (`kubectl auth whoami --as`), the `SelfSubjectRulesReview` status `kubectl auth can-i --list
//!   --as` reads, and the rows it printed.
//! - [`printed_rules`]: `kubectl auth can-i --list` rows read back as rules, for comparing a live
//!   cluster's answer with the app's (E17-S10).
//! - [`RbacScenario`]: the restricted ServiceAccount of the kind suites (E17-S10), as objects.

mod printed;
mod scenario;

pub use printed::{printed_rule, printed_rules};
pub use scenario::{ACCOUNT, POD_READER, POD_VERBS, RbacScenario, VIEW_BINDING};

use oxikube_domain::Resource;
use oxikube_domain::rbac::Subject;
use serde_json::Value;

const CLUSTER: &str = include_str!("../../../fixtures/rbac/cluster.json");
const REVIEWS: &str = include_str!("../../../fixtures/rbac/reviews.json");

/// One recorded rules review.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedReview {
    /// The subject the review was made as.
    pub subject: Subject,
    /// The user name the API server saw (for a group, the stand-in member `fx-member`).
    pub user: String,
    /// The groups the API server saw, implicit ones included.
    pub groups: Vec<String>,
    /// The namespace reviewed.
    pub namespace: String,
    /// `status` of the `SelfSubjectRulesReview`: `resourceRules`, `nonResourceRules`,
    /// `incomplete`.
    pub status: Value,
    /// The rows `kubectl auth can-i --list` printed (header dropped).
    pub can_i_list: Vec<String>,
}

/// The recorded cluster objects, sorted by kind, namespace and name.
///
/// # Panics
///
/// When the recording is not a `List` of valid objects (checked by this crate's tests).
pub fn cluster() -> Vec<Resource> {
    let list: Value = serde_json::from_str(CLUSTER).expect("rbac/cluster.json is JSON");
    list["items"]
        .as_array()
        .expect("rbac/cluster.json is a List")
        .iter()
        .map(|item| Resource::from_json(item.clone()).expect("every recorded item is an object"))
        .collect()
}

/// The recorded reviews, in recording order (each subject in `oxi-rbac-fx`, then `default`).
///
/// # Panics
///
/// When the recording is malformed (checked by this crate's tests).
pub fn reviews() -> Vec<RecordedReview> {
    let reviews: Value = serde_json::from_str(REVIEWS).expect("rbac/reviews.json is JSON");
    let text = |value: &Value, key: &str| -> String {
        value[key]
            .as_str()
            .unwrap_or_else(|| panic!("review field {key} is a string"))
            .to_owned()
    };
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    reviews
        .as_array()
        .expect("rbac/reviews.json is an array")
        .iter()
        .map(|review| {
            let kind = text(&review["subject"], "kind");
            let name = text(&review["subject"], "name");
            let subject = match kind.as_str() {
                "User" => Subject::User(name.into()),
                "Group" => Subject::Group(name.into()),
                "ServiceAccount" => {
                    let (namespace, name) = name
                        .split_once('/')
                        .expect("a ServiceAccount is namespace/name");
                    Subject::service_account(namespace, name)
                }
                other => panic!("unknown subject kind {other}"),
            };
            RecordedReview {
                subject,
                user: text(review, "user"),
                groups: strings(&review["groups"]),
                namespace: text(review, "namespace"),
                status: review["status"].clone(),
                can_i_list: strings(&review["can_i_list"]),
            }
        })
        .collect()
}
