//! The words of each state: title, hint and detail. Pure functions over [`TableState`] and the
//! [`StateLabels`] naming the kind and the scope, so the copy is unit-tested and the view only
//! lays it out.
//!
//! Server text (messages from the API server, an exec plugin, a transport) is redacted again
//! here before it is shown (non-negotiable 5), made one bounded line for the summary, and kept
//! whole (bounded) for the "Details" disclosure.

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::redact::redact;
use oxikube_domain::session::WatchScope;
use oxikube_ui::IconName;

use super::state::{Stale, TableState};

/// Longest summary line of an error.
const SHORT: usize = 160;
/// Longest detail text.
const DETAIL: usize = 2000;

/// How the table names what it lists and where, for the empty states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLabels {
    /// The kind, lowercase plural ("pods").
    pub plural: Arc<str>,
    /// The resource as RBAC names it ("pods", "deployments.apps").
    pub resource: Arc<str>,
    /// Where the table lists ("all namespaces", "namespace \"shop\"", "the cluster").
    pub scope: Arc<str>,
}

impl StateLabels {
    /// Labels for `plural` / `resource` listed in `scope`.
    pub fn new(plural: &str, resource: &str, scope: &str) -> Self {
        Self {
            plural: plural.into(),
            resource: resource.into(),
            scope: scope.into(),
        }
    }
}

/// What a state says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCopy {
    /// The icon.
    pub icon: IconName,
    /// The headline.
    pub title: String,
    /// What it means and what to do about it.
    pub hint: Option<String>,
    /// A one-line summary of the failure (shown under the hint).
    pub summary: Option<String>,
    /// The full failure text, behind "Details".
    pub detail: Option<String>,
}

impl StateCopy {
    fn new(icon: IconName, title: impl Into<String>) -> Self {
        Self {
            icon,
            title: title.into(),
            hint: None,
            summary: None,
            detail: None,
        }
    }

    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    fn failure(mut self, message: &str) -> Self {
        let summary = short(message);
        let detail = long(message);
        // A detail that says nothing more than the summary is not worth a disclosure.
        if !detail.is_empty() && detail != summary {
            self.detail = Some(detail);
        }
        if !summary.is_empty() {
            self.summary = Some(summary);
        }
        self
    }
}

/// The copy of a state with no rows.
pub fn copy(state: &TableState, labels: &StateLabels) -> StateCopy {
    let StateLabels {
        plural,
        resource,
        scope,
    } = labels;
    match state {
        TableState::Loading => StateCopy::new(IconName::LoaderCircle, format!("Loading {plural}…"))
            .hint(format!("Listing {plural} in {scope}.")),
        TableState::Reconnecting { message } => {
            StateCopy::new(IconName::Unplug, "Reconnecting to the cluster…")
                .hint(format!(
                    "The connection dropped before {plural} were listed. Oxikube keeps trying; \
                     retry now to skip the wait."
                ))
                .failure(message)
        }
        TableState::Empty => StateCopy::new(IconName::Box, format!("No {plural} in {scope}")),
        TableState::FilteredEmpty { filter } => StateCopy::new(
            IconName::Funnel,
            format!("No {plural} match “{}”", short(filter)),
        )
        .hint(format!(
            "Searched {scope}. Clear the filter to see everything."
        )),
        TableState::Forbidden { message } => {
            StateCopy::new(IconName::Lock, format!("Not allowed to list {plural}"))
                .hint(format!(
                    "The cluster denied `list` on `{resource}` in {scope}. Check your RBAC: ask a \
                     cluster admin for a Role that grants `list` and `watch` on `{resource}`, \
                     then retry."
                ))
                .failure(message)
        }
        TableState::Unauthorized { message } => {
            StateCopy::new(IconName::KeyRound, "Your credentials were rejected")
                .hint(
                    "The login for this cluster is invalid or has expired. Sign in again (renew \
                     the token or run the kubeconfig's credential plugin), then retry.",
                )
                .failure(message)
        }
        TableState::Failed { kind, message } => {
            StateCopy::new(IconName::CircleAlert, format!("Cannot list {plural}"))
                .hint(failed_hint(*kind, plural))
                .failure(message)
        }
        // The rows are the content; the badge speaks for them.
        TableState::Rows { .. } => StateCopy::new(IconName::Box, ""),
    }
}

fn failed_hint(kind: ErrorKind, plural: &str) -> String {
    match kind {
        ErrorKind::Timeout => format!("The cluster did not answer in time while listing {plural}."),
        ErrorKind::Network => format!("The cluster could not be reached while listing {plural}."),
        ErrorKind::NotFound => format!("The cluster does not serve {plural} (any more)."),
        ErrorKind::BudgetExceeded => {
            "Too many watches are open for this cluster. Close a table or raise the watch \
             budget in settings, then retry."
                .to_owned()
        }
        _ => format!("Listing {plural} failed."),
    }
}

/// The text of the stale badge: "Stale · reconnecting".
pub(super) fn stale_label(stale: &Stale) -> &'static str {
    match stale {
        Stale::Refreshing => "Stale · refreshing",
        Stale::Reconnecting { .. } => "Stale · reconnecting",
        Stale::Forbidden => "Stale · not allowed",
        Stale::Unauthorized => "Stale · sign in again",
        Stale::Failed { .. } => "Stale · feed stopped",
    }
}

/// The tooltip of the stale badge: what happened, redacted.
pub(super) fn stale_tip(stale: &Stale) -> String {
    match stale {
        Stale::Refreshing => "Listing again; these rows are from before.".to_owned(),
        Stale::Reconnecting { message } => {
            format!(
                "Reconnecting; these rows are from before. {}",
                short(message)
            )
        }
        Stale::Forbidden => {
            "You may no longer list this kind; these are the last rows seen.".into()
        }
        Stale::Unauthorized => {
            "Your credentials were rejected; these are the last rows seen.".into()
        }
        Stale::Failed { message } => {
            format!(
                "The feed stopped; these are the last rows seen. {}",
                short(message)
            )
        }
    }
}

/// `text` redacted, as one line of at most [`SHORT`] characters.
pub(super) fn short(text: &str) -> String {
    let redacted = redact(text);
    let line = redacted
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    bound(line.trim(), SHORT)
}

/// `text` redacted, whole but at most [`DETAIL`] characters.
fn long(text: &str) -> String {
    bound(redact(text).trim(), DETAIL)
}

fn bound(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// How a scope reads in the copy.
pub(in crate::table) fn scope_label(scope: &WatchScope, namespaced: bool) -> String {
    if !namespaced {
        return "the cluster".to_owned();
    }
    match scope {
        WatchScope::Cluster => "all namespaces".to_owned(),
        WatchScope::Namespaces(names) => match names.as_slice() {
            [] => "no namespace".to_owned(),
            [one] => format!("namespace “{one}”"),
            many => {
                let shown: Vec<&str> = many.iter().take(3).map(String::as_str).collect();
                let more = many.len().saturating_sub(shown.len());
                if more == 0 {
                    format!("namespaces {}", shown.join(", "))
                } else {
                    format!("namespaces {} and {more} more", shown.join(", "))
                }
            }
        },
    }
}
