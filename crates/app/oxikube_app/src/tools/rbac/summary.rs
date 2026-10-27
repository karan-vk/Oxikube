//! The compact verb summary of a ServiceAccount: a fixed set of common resources checked with
//! `SubjectAccessReview`s, a few at a time.

use futures::future::join_all;
use oxikube_domain::OxiResult;
use oxikube_domain::access::{AccessCheck, AccessDecision, Impersonate};
use oxikube_domain::ids::Gvr;
use oxikube_ports::AccessReviewPort;

/// The verbs summarised per resource.
const VERBS: [&str; 5] = ["get", "list", "create", "patch", "delete"];

/// Reviews in flight at once.
const IN_FLIGHT: usize = 4;

/// Common resources: group, plural, whether namespaced.
const RESOURCES: [(&str, &str, bool); 12] = [
    ("", "pods", true),
    ("apps", "deployments", true),
    ("apps", "statefulsets", true),
    ("", "services", true),
    ("", "configmaps", true),
    ("", "secrets", true),
    ("batch", "jobs", true),
    ("networking.k8s.io", "ingresses", true),
    ("", "persistentvolumeclaims", true),
    ("", "nodes", false),
    ("", "namespaces", false),
    ("", "persistentvolumes", false),
];

/// One resource's allowed verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    /// `pods`, `deployments.apps`.
    pub(super) resource: String,
    /// The verbs the review allowed.
    pub(super) allowed: Vec<&'static str>,
    /// The verbs whose review was inconclusive (never shown as denied).
    pub(super) unknown: Vec<&'static str>,
}

/// What the reviews said.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Summary {
    /// Whether `*` on `*` is allowed (cluster-admin-like).
    pub(super) everything: bool,
    /// Whether the `*` on `*` review was inconclusive.
    pub(super) everything_unknown: bool,
    /// One row per resource.
    pub(super) rows: Vec<Row>,
    /// Why the summary is missing or incomplete (no permission to review as another subject).
    pub(super) unavailable: Option<String>,
}

/// One review's outcome; unknown is kept apart from denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Allowed,
    Denied,
    Unknown,
}

/// One review; a named function so the stream's closure is not higher-ranked over the borrow.
async fn review(port: &dyn AccessReviewPort, check: &AccessCheck) -> OxiResult<AccessDecision> {
    port.can_i(check).await
}

/// Checks `subject` against the common resources in `namespace`.
pub(super) async fn summarize(
    port: &dyn AccessReviewPort,
    subject: &Impersonate,
    namespace: &str,
) -> Summary {
    let mut checks =
        vec![AccessCheck::new("*", Gvr::new("*", "", "*")).impersonating(subject.clone())];
    for (group, plural, namespaced) in RESOURCES {
        for verb in VERBS {
            let mut check =
                AccessCheck::new(verb, Gvr::new(group, "", plural)).impersonating(subject.clone());
            if namespaced {
                check = check.namespace(namespace);
            }
            checks.push(check);
        }
    }
    let mut answers: Vec<OxiResult<AccessDecision>> = Vec::with_capacity(checks.len());
    for chunk in checks.chunks(IN_FLIGHT) {
        answers.extend(join_all(chunk.iter().map(|check| review(port, check))).await);
    }
    let mut summary = Summary::default();
    if let Some(Err(error)) = answers.iter().find(|a| a.is_err()) {
        summary.unavailable = Some(error.message().to_owned());
        return summary;
    }
    let verdicts: Vec<Verdict> = answers
        .iter()
        .map(|a| match a {
            Ok(AccessDecision::Allowed) => Verdict::Allowed,
            Ok(AccessDecision::Unknown { .. }) => Verdict::Unknown,
            _ => Verdict::Denied,
        })
        .collect();
    summary.everything = verdicts[0] == Verdict::Allowed;
    summary.everything_unknown = verdicts[0] == Verdict::Unknown;
    for (i, (group, plural, _)) in RESOURCES.iter().enumerate() {
        let start = 1 + i * VERBS.len();
        let answered = VERBS.iter().zip(&verdicts[start..start + VERBS.len()]);
        let verbs_with = |wanted: Verdict| {
            answered
                .clone()
                .filter(|(_, verdict)| **verdict == wanted)
                .map(|(verb, _)| *verb)
                .collect::<Vec<_>>()
        };
        let resource = if group.is_empty() {
            (*plural).to_owned()
        } else {
            format!("{plural}.{group}")
        };
        summary.rows.push(Row {
            resource,
            allowed: verbs_with(Verdict::Allowed),
            unknown: verbs_with(Verdict::Unknown),
        });
    }
    summary
}
