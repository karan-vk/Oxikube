//! Pure conversion from kube discovery documents to domain [`ResourceKind`]s.
//!
//! kube's own `ApiResource` / `ApiCapabilities` drop short names and categories, so the adapter
//! reads the raw documents (`APIResourceDiscovery` from aggregated discovery, `APIResource` from
//! the legacy per-group lists) and builds both the domain record and the `ApiResource` E04 needs
//! for dynamic `Api` handles. Nothing here does I/O.
//!
//! Rules shared by both shapes: subresources (`pods/log`) are skipped, kinds without `get` or
//! `list` are dropped (`Binding`, `TokenReview`), verbs this model does not carry are ignored,
//! and every served version is kept with `preferred` set on the group's preferred one.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::{APIGroup, APIResourceList};
use kube::core::discovery::v2::APIGroupDiscovery;
use kube::core::{ApiResource, Version};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, Verb, VerbSet};
use tracing::debug;

/// One served kind: the domain record plus the kube handle E04 builds `Api`s from.
#[derive(Debug, Clone)]
pub(crate) struct Discovered {
    pub(crate) kind: ResourceKind,
    pub(crate) api_resource: ApiResource,
}

/// One `APIResourceList` from the legacy shape, with whether its version is the preferred one.
pub(crate) struct LegacyList {
    pub(crate) preferred: bool,
    pub(crate) list: APIResourceList,
}

/// Source-independent view of one resource entry.
struct Raw<'a> {
    group: &'a str,
    version: &'a str,
    preferred: bool,
    plural: &'a str,
    singular: &'a str,
    kind: &'a str,
    namespaced: bool,
    verbs: &'a [String],
    short_names: &'a [String],
    categories: &'a [String],
}

/// Converts aggregated discovery (`apidiscovery.k8s.io/v2`) groups. The core group is the item
/// without a name. Versions arrive in descending preference, so the first is the preferred one.
pub(crate) fn from_aggregated(groups: &[APIGroupDiscovery]) -> Vec<Discovered> {
    let mut out = Vec::new();
    for group in groups {
        let name = group
            .metadata
            .as_ref()
            .and_then(|m| m.name.as_deref())
            .unwrap_or_default();
        for (index, version) in group.versions.iter().enumerate() {
            let Some(version_name) = version.version.as_deref().filter(|v| !v.is_empty()) else {
                continue;
            };
            for res in &version.resources {
                let kind = res.response_kind.as_ref().and_then(|k| k.kind.as_deref());
                let raw = Raw {
                    group: name,
                    version: version_name,
                    preferred: index == 0,
                    plural: res.resource.as_deref().unwrap_or_default(),
                    singular: res.singular_resource.as_deref().unwrap_or_default(),
                    kind: kind.unwrap_or_default(),
                    namespaced: res.scope.as_deref() == Some("Namespaced"),
                    verbs: &res.verbs,
                    short_names: &res.short_names,
                    categories: &res.categories,
                };
                out.extend(build(raw));
            }
        }
    }
    out
}

/// Converts legacy `APIResourceList`s (`/api/v1`, `/apis/<group>/<version>`).
pub(crate) fn from_legacy(lists: &[LegacyList]) -> Vec<Discovered> {
    let mut out = Vec::new();
    for LegacyList { preferred, list } in lists {
        let (list_group, list_version) = split_group_version(&list.group_version);
        for res in &list.resources {
            let raw = Raw {
                group: res.group.as_deref().unwrap_or(list_group),
                version: res.version.as_deref().unwrap_or(list_version),
                preferred: *preferred,
                plural: &res.name,
                singular: &res.singular_name,
                kind: &res.kind,
                namespaced: res.namespaced,
                verbs: &res.verbs,
                short_names: res.short_names.as_deref().unwrap_or_default(),
                categories: res.categories.as_deref().unwrap_or_default(),
            };
            out.extend(build(raw));
        }
    }
    out
}

/// The preferred version of a legacy `APIGroup`: the server's choice, else the highest priority
/// version by Kubernetes version ordering (`v2` > `v1` > `v1beta1` > `v1alpha1`).
pub(crate) fn preferred_version(group: &APIGroup) -> Option<String> {
    if let Some(preferred) = &group.preferred_version {
        return Some(preferred.version.clone());
    }
    group
        .versions
        .iter()
        .max_by_key(|v| Version::parse(&v.version).priority())
        .map(|v| v.version.clone())
}

/// Splits an `apiVersion` (`apps/v1`, `v1`) into `(group, version)`.
fn split_group_version(group_version: &str) -> (&str, &str) {
    group_version.split_once('/').unwrap_or(("", group_version))
}

fn build(raw: Raw<'_>) -> Option<Discovered> {
    if raw.plural.is_empty() || raw.kind.is_empty() {
        return None;
    }
    if raw.plural.contains('/') {
        debug!(resource = raw.plural, "discovery: skipping subresource");
        return None;
    }
    let verbs = VerbSet::from_names(raw.verbs.iter().map(String::as_str));
    if !verbs.contains(Verb::Get) && !verbs.contains(Verb::List) {
        debug!(
            group = raw.group,
            kind = raw.kind,
            "discovery: skipping kind without get/list"
        );
        return None;
    }
    let api_version = if raw.group.is_empty() {
        raw.version.to_owned()
    } else {
        format!("{}/{}", raw.group, raw.version)
    };
    let api_resource = ApiResource {
        group: raw.group.to_owned(),
        version: raw.version.to_owned(),
        api_version,
        kind: raw.kind.to_owned(),
        plural: raw.plural.to_owned(),
    };
    let kind = ResourceKind {
        gvk: Gvk::new(raw.group, raw.version, raw.kind),
        preferred: raw.preferred,
        plural: raw.plural.to_owned(),
        singular: raw.singular.to_owned(),
        short_names: raw.short_names.to_vec(),
        categories: raw.categories.to_vec(),
        verbs,
        namespaced: raw.namespaced,
    };
    Some(Discovered { kind, api_resource })
}
