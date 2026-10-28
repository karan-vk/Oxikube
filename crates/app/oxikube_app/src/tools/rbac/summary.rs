//! The compact verb summary of a ServiceAccount: a fixed set of common resources checked with
//! `SubjectAccessReview`s, a few at a time.
//!
//! The `*` on `*` review goes first and alone: when it is refused (the caller may not create
//! `SubjectAccessReview`s) or allowed (cluster-admin), the other 60 would add nothing, so they are
//! not sent. Otherwise they run [`IN_FLIGHT`] at a time as a stream (a slow review holds up only
//! its own slot), and the first error stops the rest.

use futures::FutureExt as _;
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt as _};
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
    let mut summary = Summary::default();
    let everything = AccessCheck::new("*", Gvr::new("*", "", "*")).impersonating(subject.clone());
    match review(port, &everything).await {
        Err(error) => {
            summary.unavailable = Some(error.message().to_owned());
            return summary;
        }
        Ok(AccessDecision::Allowed) => {
            summary.everything = true;
            summary.rows = RESOURCES
                .iter()
                .map(|(group, plural, _)| Row {
                    resource: resource_label(group, plural),
                    allowed: VERBS.to_vec(),
                    unknown: Vec::new(),
                })
                .collect();
            return summary;
        }
        Ok(AccessDecision::Unknown { .. }) => summary.everything_unknown = true,
        Ok(AccessDecision::Denied { .. }) => {}
    }
    let mut checks = Vec::with_capacity(RESOURCES.len() * VERBS.len());
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
    // Boxed so the stream's item is not a closure higher-ranked over the borrow of `checks`.
    let reviews: Vec<BoxFuture<'_, OxiResult<AccessDecision>>> = checks
        .iter()
        .map(|check| review(port, check).boxed())
        .collect();
    let mut answers = stream::iter(reviews).buffered(IN_FLIGHT);
    let mut verdicts = Vec::with_capacity(checks.len());
    while let Some(answer) = answers.next().await {
        verdicts.push(match answer {
            Err(error) => {
                summary.unavailable = Some(error.message().to_owned());
                return summary;
            }
            Ok(AccessDecision::Allowed) => Verdict::Allowed,
            Ok(AccessDecision::Unknown { .. }) => Verdict::Unknown,
            Ok(AccessDecision::Denied { .. }) => Verdict::Denied,
        });
    }
    for ((group, plural, _), answered) in RESOURCES.iter().zip(verdicts.chunks(VERBS.len())) {
        let verbs_with = |wanted: Verdict| {
            VERBS
                .iter()
                .zip(answered)
                .filter(|(_, verdict)| **verdict == wanted)
                .map(|(verb, _)| *verb)
                .collect::<Vec<_>>()
        };
        summary.rows.push(Row {
            resource: resource_label(group, plural),
            allowed: verbs_with(Verdict::Allowed),
            unknown: verbs_with(Verdict::Unknown),
        });
    }
    summary
}

/// `pods`, `deployments.apps`.
fn resource_label(group: &str, plural: &str) -> String {
    if group.is_empty() {
        plural.to_owned()
    } else {
        format!("{plural}.{group}")
    }
}
