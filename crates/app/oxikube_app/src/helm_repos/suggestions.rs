//! Quick-add suggestions: well-known repositories offered, never added on their own.
//!
//! E16-S11 defines the `helm.default_repos` setting; until it lands this is its default value.

use oxikube_domain::helm::RepoEntry;

/// A repository the add form can fill in with one click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoSuggestion {
    /// The alias.
    pub name: &'static str,
    /// The index URL.
    pub url: &'static str,
}

const DEFAULTS: &[RepoSuggestion] = &[
    RepoSuggestion {
        name: "bitnami",
        url: "https://charts.bitnami.com/bitnami",
    },
    RepoSuggestion {
        name: "ingress-nginx",
        url: "https://kubernetes.github.io/ingress-nginx",
    },
    RepoSuggestion {
        name: "prometheus-community",
        url: "https://prometheus-community.github.io/helm-charts",
    },
    RepoSuggestion {
        name: "jetstack",
        url: "https://charts.jetstack.io",
    },
];

/// The suggestions that are not configured yet (same alias or same URL).
pub fn default_suggestions(configured: &[RepoEntry]) -> Vec<RepoSuggestion> {
    DEFAULTS
        .iter()
        .filter(|s| {
            !configured
                .iter()
                .any(|e| e.name == s.name || e.url.trim_end_matches('/') == s.url)
        })
        .copied()
        .collect()
}
