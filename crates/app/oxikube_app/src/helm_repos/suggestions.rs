//! Quick-add suggestions: well-known repositories offered, never added on their own.
//!
//! The list is the user's `helm.default_repos` setting (E16-S11; validated by
//! `oxikube_settings::HelmSettings`), read by the pages at render time so an edit applies without
//! a restart. An empty setting offers nothing.

use oxikube_domain::helm::RepoEntry;

/// A repository the add form can fill in with one click.
pub type RepoSuggestion = RepoEntry;

/// The entries of `defaults` (the `helm.default_repos` setting) that are not configured yet
/// (same alias or same URL), in the setting's order.
pub fn default_suggestions(
    defaults: &[RepoEntry],
    configured: &[RepoEntry],
) -> Vec<RepoSuggestion> {
    let same_url = |a: &str, b: &str| a.trim_end_matches('/') == b.trim_end_matches('/');
    defaults
        .iter()
        .filter(|s| {
            !configured
                .iter()
                .any(|e| e.name == s.name || same_url(&e.url, &s.url))
        })
        .cloned()
        .collect()
}
