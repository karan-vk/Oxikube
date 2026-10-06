//! [`review_access`]: the rules the sidebar gates its sections on.

use futures::future::join_all;
use oxikube_domain::access::{AccessRequirement, AccessRules};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::AccessReviewPort;

/// What the rules review said, as the sidebar uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessOutcome {
    /// The reviews succeeded: the union of the rules of every selected namespace. Under
    /// [`NamespaceSelection::All`] the rules are marked partial (see [`review_access`]).
    Reviewed(AccessRules),
    /// A review failed: everything is offered and `reason` is shown as a warning.
    Failed {
        /// Why, from the adapter's (already redacted) error message.
        reason: String,
    },
}

impl AccessOutcome {
    /// Whether an entry needing any of `requirements` is shown. Always `true` for
    /// [`AccessOutcome::Failed`] (fail open).
    pub fn offers(&self, requirements: &[AccessRequirement]) -> bool {
        match self {
            AccessOutcome::Reviewed(rules) => rules.any_offered(requirements),
            AccessOutcome::Failed { .. } => true,
        }
    }

    /// The warning to show next to the sidebar, if the review failed.
    pub fn warning(&self) -> Option<&str> {
        match self {
            AccessOutcome::Reviewed(_) => None,
            AccessOutcome::Failed { reason } => Some(reason),
        }
    }
}

/// Runs the rules reviews for `selection` and merges them. Read-only (a rules review creates
/// nothing), so it needs no `MutationGuard`.
///
/// * A selection of named namespaces asks for each of them and nothing else. Cluster-scoped grants
///   appear in every namespace's answer, and asking cluster-wide as well would merge in the
///   namespaced grants of whichever namespace the adapter probes with, showing sections for
///   namespaces nobody selected.
/// * `All` has no namespace to ask in: the cluster-wide review sees the cluster-scoped grants but
///   only one probe namespace's Roles, so a user bound only in `dev` looks denied everywhere. The
///   merged rules are therefore marked partial: what the review did not list is unknown, not
///   denied, and the sidebar keeps the section (hiding by mistake is worse than showing a page the
///   server refuses). Selecting a namespace gives the precise answer.
///
/// Never fails: an error from the port becomes [`AccessOutcome::Failed`], so the sidebar shows
/// everything with a warning.
pub async fn review_access(
    access: &dyn AccessReviewPort,
    selection: &NamespaceSelection,
) -> AccessOutcome {
    let asks: Vec<Option<&str>> = match selection {
        NamespaceSelection::All => vec![None],
        NamespaceSelection::Set(names) => names.iter().map(|name| Some(name.as_str())).collect(),
    };
    let answers = join_all(asks.iter().map(|ns| access.rules(*ns))).await;
    let mut merged = AccessRules::none();
    for (ask, answer) in asks.iter().zip(answers) {
        match answer {
            Ok(rules) => merged.merge(rules),
            Err(error) => {
                tracing::warn!(namespace = ?ask, %error, "rules review failed; showing every section");
                return AccessOutcome::Failed {
                    reason: error.message().to_owned(),
                };
            }
        }
    }
    if selection.is_all() {
        merged.partial = true;
    }
    AccessOutcome::Reviewed(merged)
}
