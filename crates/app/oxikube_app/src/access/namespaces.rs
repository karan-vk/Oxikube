//! [`accessible_namespaces`]: the namespaces to offer when the cluster-wide list is forbidden.

use std::collections::BTreeSet;

use futures::stream::{self, StreamExt};
use oxikube_domain::access::{AccessRules, is_cluster_scoped};
use oxikube_domain::session::NamespaceSelection;

use super::service::{AccessService, MAX_IN_FLIGHT};

/// Where the accessible namespaces came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespaceOrigin {
    /// The cluster's `accessible_namespaces` setting (Lens `accessibleNamespaces`): used as
    /// given, nothing is reviewed.
    Override,
    /// Candidate namespaces whose rules review shows the user may read something.
    Reviewed,
}

/// The namespaces a restricted user can work in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibleNamespaces {
    /// Sorted, unique names.
    pub names: Vec<String>,
    /// How they were found.
    pub origin: NamespaceOrigin,
}

impl AccessibleNamespaces {
    /// The names as a selection the namespace selector can apply (`All` when there are none).
    pub fn selection(&self) -> NamespaceSelection {
        NamespaceSelection::from_names(&self.names)
    }
}

/// Whether `rules` let the user `get`, `list` or `watch` something namespaced: cluster-scoped
/// grants appear in every namespace's review, so they prove nothing about the namespace.
fn reads_something_namespaced(rules: &AccessRules) -> bool {
    rules.rules.iter().any(|rule| {
        let reads = rule
            .verbs
            .iter()
            .any(|verb| matches!(verb.as_str(), "*" | "get" | "list" | "watch"));
        reads
            && rule.resources.iter().any(|resource| {
                resource == "*"
                    || rule
                        .api_groups
                        .iter()
                        .any(|group| !is_cluster_scoped(group, resource))
            })
    })
}

/// Computes the accessible namespaces.
///
/// A non-empty `override_names` (the cluster's `accessible_namespaces` setting) is the answer.
/// Otherwise each of `candidates` (the kubeconfig context's namespace, favourites, names the user
/// typed, the current selection) gets a rules review through `service` (cached, at most
/// [`MAX_IN_FLIGHT`] at once) and the ones where the user may read something are kept. A review
/// that fails keeps its candidate: this is an offer to pick from, never a hiding gate.
pub async fn accessible_namespaces(
    service: &AccessService,
    override_names: &[String],
    candidates: impl IntoIterator<Item = String>,
) -> AccessibleNamespaces {
    let own = |names: BTreeSet<String>| names.into_iter().collect::<Vec<_>>();
    let configured: BTreeSet<String> = override_names
        .iter()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    if !configured.is_empty() {
        return AccessibleNamespaces {
            names: own(configured),
            origin: NamespaceOrigin::Override,
        };
    }
    let candidates: BTreeSet<String> = candidates
        .into_iter()
        .filter(|name| !name.is_empty())
        .collect();
    let kept: BTreeSet<String> = stream::iter(candidates)
        .map(|name| async move {
            let keep = match service.rules(Some(&name)).await {
                Ok(rules) => reads_something_namespaced(&rules),
                Err(_) => true,
            };
            keep.then_some(name)
        })
        .buffer_unordered(MAX_IN_FLIGHT)
        .filter_map(|name| async move { name })
        .collect()
        .await;
    AccessibleNamespaces {
        names: own(kept),
        origin: NamespaceOrigin::Reviewed,
    }
}
