//! The `ResourceKind` registry record: what API discovery tells us about one
//! Kubernetes type.
//!
//! Discovery (E03-S06) fills these in, and every table, palette and sidebar
//! reads them. The record is plain data with no dependency on kube-rs; the
//! mapping from discovery output lives in `oxikube_kube`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ids::{Gvk, Gvr, IdParseError, Scope};

/// An API verb a resource supports, as listed by discovery.
///
/// Only the standard CRUD and watch verbs are modelled. Discovery may also
/// report subresource or authorization verbs (`proxy`, `bind`, `impersonate`,
/// ...); [`Verb::parse`] returns `None` for those and callers drop them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verb {
    /// Read one object.
    Get,
    /// List objects.
    List,
    /// Stream changes.
    Watch,
    /// Create an object.
    Create,
    /// Replace an object.
    Update,
    /// Patch an object.
    Patch,
    /// Delete one object.
    Delete,
    /// Delete a collection of objects.
    DeleteCollection,
}

impl Verb {
    /// Every modelled verb, in declaration order.
    pub const ALL: [Verb; 8] = [
        Verb::Get,
        Verb::List,
        Verb::Watch,
        Verb::Create,
        Verb::Update,
        Verb::Patch,
        Verb::Delete,
        Verb::DeleteCollection,
    ];

    /// The Kubernetes spelling of the verb (`deletecollection`, not `delete_collection`).
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Get => "get",
            Verb::List => "list",
            Verb::Watch => "watch",
            Verb::Create => "create",
            Verb::Update => "update",
            Verb::Patch => "patch",
            Verb::Delete => "delete",
            Verb::DeleteCollection => "deletecollection",
        }
    }

    /// Parse a discovery verb; `None` for verbs this model does not carry.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// Bit used by [`VerbSet`].
    const fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A small, `Copy` set of [`Verb`]s (one byte).
///
/// Iteration and serialization are in [`Verb`] declaration order, so the JSON
/// form is a stable list such as `["get","list","watch"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VerbSet(u8);

impl VerbSet {
    /// The empty set.
    pub const EMPTY: VerbSet = VerbSet(0);

    /// Add a verb.
    pub fn insert(&mut self, verb: Verb) {
        self.0 |= verb.bit();
    }

    /// Whether the set contains `verb`.
    pub fn contains(self, verb: Verb) -> bool {
        self.0 & verb.bit() != 0
    }

    /// Whether the set has no verbs.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Number of verbs in the set.
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// The verbs in declaration order.
    pub fn iter(self) -> impl Iterator<Item = Verb> {
        Verb::ALL.into_iter().filter(move |v| self.contains(*v))
    }

    /// Build a set from discovery's verb strings, skipping verbs this model
    /// does not carry.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        names.into_iter().filter_map(Verb::parse).collect()
    }
}

impl FromIterator<Verb> for VerbSet {
    fn from_iter<T: IntoIterator<Item = Verb>>(iter: T) -> Self {
        let mut set = VerbSet::EMPTY;
        for verb in iter {
            set.insert(verb);
        }
        set
    }
}

impl<const N: usize> From<[Verb; N]> for VerbSet {
    fn from(verbs: [Verb; N]) -> Self {
        verbs.into_iter().collect()
    }
}

impl Serialize for VerbSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for VerbSet {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Vec::<Verb>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

/// Registry record for one Kubernetes type, built from API discovery.
///
/// `gvk` carries the group, version and kind this record describes. Names that
/// kubectl accepts (plural, singular, short names) and `categories` (such as
/// `all`) come straight from the discovery `APIResource`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceKind {
    /// The type described, including the served version.
    pub gvk: Gvk,
    /// Whether `gvk.version` is the preferred served version of this kind: the group's
    /// preferred version, or the highest-priority version for a kind the preferred one lacks.
    pub preferred: bool,
    /// Plural resource name used in REST paths, for example `deployments`.
    pub plural: String,
    /// Singular name, for example `deployment`. May be empty if discovery omits it.
    #[serde(default)]
    pub singular: String,
    /// Short names, for example `deploy`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub short_names: Vec<String>,
    /// Categories the resource belongs to, for example `all`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Verbs the API server supports for this resource.
    pub verbs: VerbSet,
    /// Whether objects live in a namespace. [`ResourceKind::scope`] is derived
    /// from this, so the two cannot disagree.
    pub namespaced: bool,
}

impl ResourceKind {
    /// [`Scope::Namespaced`] or [`Scope::Cluster`], derived from `namespaced`.
    pub fn scope(&self) -> Scope {
        Scope::from_namespaced(self.namespaced)
    }

    /// The REST identity: the same group and version with the plural name.
    pub fn gvr(&self) -> Gvr {
        Gvr::new(
            self.gvk.group.clone(),
            self.gvk.version.clone(),
            self.plural.as_str(),
        )
    }

    /// Whether the server supports `verb` for this resource.
    pub fn supports(&self, verb: Verb) -> bool {
        self.verbs.contains(verb)
    }

    /// Whether a live feed is possible: the resource can be both listed and watched.
    pub fn is_watchable(&self) -> bool {
        self.supports(Verb::List) && self.supports(Verb::Watch)
    }

    /// Whether `name` refers to this kind the way kubectl resolves it:
    /// plural, singular, a short name, or the kind, compared case-insensitively.
    pub fn matches_name(&self, name: &str) -> bool {
        let eq = |candidate: &str| !candidate.is_empty() && candidate.eq_ignore_ascii_case(name);
        eq(&self.plural)
            || eq(&self.singular)
            || eq(&self.gvk.kind)
            || self.short_names.iter().any(|s| eq(s))
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.gvk, f)
    }
}

impl FromStr for Verb {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Verb::parse(s).ok_or_else(|| IdParseError::Malformed {
            what: "Verb",
            input: s.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployment() -> ResourceKind {
        ResourceKind {
            gvk: Gvk::new("apps", "v1", "Deployment"),
            preferred: true,
            plural: "deployments".into(),
            singular: "deployment".into(),
            short_names: vec!["deploy".into()],
            categories: vec!["all".into()],
            verbs: VerbSet::from_names([
                "create",
                "delete",
                "deletecollection",
                "get",
                "list",
                "patch",
                "update",
                "watch",
            ]),
            namespaced: true,
        }
    }

    fn bare_node() -> ResourceKind {
        ResourceKind {
            gvk: Gvk::new("", "v1", "Node"),
            preferred: true,
            plural: "nodes".into(),
            singular: String::new(),
            short_names: Vec::new(),
            categories: Vec::new(),
            verbs: [Verb::Get, Verb::List, Verb::Watch].into(),
            namespaced: false,
        }
    }

    #[test]
    fn serde_round_trip_with_shortnames_and_categories() {
        let kind = deployment();
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<ResourceKind>(&json).unwrap(), kind);
        assert!(json.contains(r#""short_names":["deploy"]"#));
        assert!(json.contains(r#""categories":["all"]"#));
        assert!(json.contains(
            r#""verbs":["get","list","watch","create","update","patch","delete","deletecollection"]"#
        ));
    }

    #[test]
    fn serde_round_trip_without_shortnames_and_categories() {
        let kind = bare_node();
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<ResourceKind>(&json).unwrap(), kind);
        assert!(!json.contains("short_names"));
        assert!(!json.contains("categories"));
        // Fixtures may omit the optional fields entirely.
        let minimal = r#"{"gvk":{"version":"v1","kind":"Node"},"preferred":true,
            "plural":"nodes","verbs":["get"],"namespaced":false}"#;
        let parsed: ResourceKind = serde_json::from_str(minimal).unwrap();
        assert!(parsed.short_names.is_empty() && parsed.categories.is_empty());
        assert!(parsed.singular.is_empty());
    }

    #[test]
    fn verb_serde_and_parse() {
        for verb in Verb::ALL {
            let json = serde_json::to_string(&verb).unwrap();
            assert_eq!(json, format!("\"{}\"", verb.as_str()));
            assert_eq!(serde_json::from_str::<Verb>(&json).unwrap(), verb);
            assert_eq!(verb.as_str().parse::<Verb>().unwrap(), verb);
            assert_eq!(verb.to_string(), verb.as_str());
        }
        assert_eq!(Verb::parse("proxy"), None);
        assert!("proxy".parse::<Verb>().is_err());
    }

    #[test]
    fn verb_set_behaves_like_a_set() {
        let mut set = VerbSet::EMPTY;
        assert!(set.is_empty());
        set.insert(Verb::Watch);
        set.insert(Verb::Get);
        set.insert(Verb::Get);
        assert_eq!(set.len(), 2);
        assert!(set.contains(Verb::Get) && !set.contains(Verb::List));
        assert_eq!(set.iter().collect::<Vec<_>>(), [Verb::Get, Verb::Watch]);
        // Unknown verbs are ignored; order of input does not matter.
        assert_eq!(VerbSet::from_names(["watch", "proxy", "get"]), set);
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"["get","watch"]"#);
        assert_eq!(serde_json::from_str::<VerbSet>(&json).unwrap(), set);
    }

    #[test]
    fn scope_follows_namespaced() {
        assert_eq!(deployment().scope(), Scope::Namespaced);
        assert_eq!(bare_node().scope(), Scope::Cluster);
    }

    #[test]
    fn gvr_uses_plural_and_same_group_version() {
        assert_eq!(deployment().gvr().to_string(), "apps/v1/deployments");
        assert_eq!(bare_node().gvr().to_string(), "v1/nodes");
        assert_eq!(deployment().to_string(), "apps/v1/Deployment");
    }

    #[test]
    fn name_matching_and_verb_queries() {
        let kind = deployment();
        for name in [
            "deployments",
            "deployment",
            "Deployment",
            "deploy",
            "DEPLOY",
        ] {
            assert!(kind.matches_name(name), "{name}");
        }
        assert!(!kind.matches_name("pods"));
        assert!(!kind.matches_name(""));
        // A missing singular never matches the empty string.
        assert!(!bare_node().matches_name(""));
        assert!(bare_node().matches_name("node"));
        assert!(kind.is_watchable() && kind.supports(Verb::Patch));
        let no_watch = ResourceKind {
            verbs: [Verb::Get, Verb::List].into(),
            ..kind
        };
        assert!(!no_watch.is_watchable());
    }
}
