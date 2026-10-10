//! The discovery layer: what the cluster serves, kept by API group so a CRD change replaces one
//! group and leaves the rest alone.
//!
//! For every served type the layer contributes its plural, singular, short names and lower-cased
//! Kind, plus `plural.group` for a type outside the core group (`certificates.cert-manager.io`,
//! the way kubectl writes it), which is unambiguous when two groups share a plural. All of them
//! lead to the type's preferred served version.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;

use super::entry::{AliasEntry, AliasSource};

/// One API group's served types and the aliases derived from them.
#[derive(Default)]
struct Group {
    /// Types by plural. One per plural: the version the server prefers.
    kinds: BTreeMap<Arc<str>, ResourceKind>,
    /// The aliases of `kinds`, rebuilt when the group changes.
    entries: Vec<AliasEntry>,
}

impl Group {
    fn rebuild_entries(&mut self) {
        self.entries.clear();
        for kind in self.kinds.values() {
            push_entries(kind, &mut self.entries);
        }
    }
}

/// Every served type, by group. The core group is `""` and sorts first, so iteration order never
/// depends on the order discovery reported things in.
#[derive(Default)]
pub(super) struct Discovered {
    groups: BTreeMap<Arc<str>, Group>,
}

impl Discovered {
    /// Replaces everything with `kinds`.
    pub(super) fn replace_all(&mut self, kinds: &[ResourceKind]) {
        self.groups.clear();
        for kind in kinds {
            insert(&mut self.groups, kind, false);
        }
        for group in self.groups.values_mut() {
            group.rebuild_entries();
        }
    }

    /// Adds or replaces the types in `kinds`; only their groups are rebuilt.
    pub(super) fn upsert(&mut self, kinds: &[ResourceKind]) {
        let mut touched: Vec<Arc<str>> = Vec::new();
        for kind in kinds {
            if let Some(group) = insert(&mut self.groups, kind, true)
                && !touched.contains(&group)
            {
                touched.push(group);
            }
        }
        for group in touched {
            if let Some(group) = self.groups.get_mut(&group) {
                group.rebuild_entries();
            }
        }
    }

    /// Forgets the types of `removed` (matched by group and Kind); only their groups are
    /// rebuilt, and a group left empty goes away.
    pub(super) fn remove(&mut self, removed: &[Gvk]) {
        for gvk in removed {
            let Some(group) = self.groups.get_mut(&*gvk.group) else {
                continue;
            };
            let before = group.kinds.len();
            group.kinds.retain(|_, kind| kind.gvk.kind != gvk.kind);
            if group.kinds.len() != before {
                group.rebuild_entries();
            }
            if group.kinds.is_empty() {
                self.groups.remove(&*gvk.group);
            }
        }
    }

    /// Forgets everything (the cluster disconnected).
    pub(super) fn clear(&mut self) {
        self.groups.clear();
    }

    /// The version the server prefers for `resource` in `group`, if it serves it.
    pub(super) fn served_version(&self, group: &str, resource: &str) -> Option<Arc<str>> {
        self.groups
            .get(group)?
            .kinds
            .get(resource)
            .map(|kind| kind.gvk.version.clone())
    }

    /// Every served type, group by group.
    pub(super) fn kinds(&self) -> impl Iterator<Item = &ResourceKind> {
        self.groups.values().flat_map(|group| group.kinds.values())
    }

    /// Every discovery alias, group by group.
    pub(super) fn entries(&self) -> impl Iterator<Item = &AliasEntry> {
        self.groups.values().flat_map(|group| group.entries.iter())
    }

    /// How many types are served.
    #[cfg(test)]
    pub(super) fn kind_count(&self) -> usize {
        self.groups.values().map(|g| g.kinds.len()).sum()
    }
}

/// Inserts `kind` unless a better version of the same type is there. Returns its group when the
/// group changed.
fn insert(
    groups: &mut BTreeMap<Arc<str>, Group>,
    kind: &ResourceKind,
    live: bool,
) -> Option<Arc<str>> {
    if kind.plural.is_empty() {
        return None;
    }
    let group = groups.entry(kind.gvk.group.clone()).or_default();
    let plural: Arc<str> = Arc::from(kind.plural.as_str());
    match group.kinds.get(&plural) {
        Some(current) if !better(kind, current, live) => None,
        _ => {
            group.kinds.insert(plural, kind.clone());
            Some(kind.gvk.group.clone())
        }
    }
}

/// Whether `candidate` should replace `current` as the record of one type.
///
/// The version the server marks preferred wins. Between equals, a `live` update (a CRD changed
/// while connected) is newer than what is held, so it replaces it; in a full listing the more
/// stable and newer version wins (`v1` over `v1beta2` over `v1beta1` over `v1alpha1`) and a
/// repeat changes nothing, which keeps the result independent of the listing's order.
fn better(candidate: &ResourceKind, current: &ResourceKind, live: bool) -> bool {
    if candidate.preferred != current.preferred {
        return candidate.preferred;
    }
    if live {
        return candidate != current;
    }
    let (new, old) = (
        version_rank(&candidate.gvk.version),
        version_rank(&current.gvk.version),
    );
    // Equal ranks only happen when a listing repeats a version with different details, which a
    // server does not do; a fixed tie-break keeps even that independent of the order.
    new > old
        || (new == old
            && (&candidate.short_names, &candidate.singular)
                > (&current.short_names, &current.singular))
}

/// Orders Kubernetes API versions: GA above beta above alpha, then the higher number, then the
/// higher beta/alpha revision. Unparseable versions rank lowest.
fn version_rank(version: &str) -> (u8, u32, u32) {
    let Some(rest) = version.strip_prefix('v') else {
        return (0, 0, 0);
    };
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let Ok(major) = rest[..digits_end].parse::<u32>() else {
        return (0, 0, 0);
    };
    let tail = &rest[digits_end..];
    if tail.is_empty() {
        return (3, major, 0);
    }
    let (stability, revision) = if let Some(n) = tail.strip_prefix("beta") {
        (2, n)
    } else if let Some(n) = tail.strip_prefix("alpha") {
        (1, n)
    } else {
        return (0, 0, 0);
    };
    (stability, major, revision.parse().unwrap_or(0))
}

/// Appends the aliases of one served type.
fn push_entries(kind: &ResourceKind, out: &mut Vec<AliasEntry>) {
    let target = AliasTarget::Gvr(kind.gvr());
    let first = out.len();
    let mut add = |name: &str| {
        if name.is_empty() || name.contains(char::is_whitespace) {
            return;
        }
        let name = name.to_ascii_lowercase();
        // A type often has the same word as singular, Kind and short name.
        if out[first..].iter().any(|e| &*e.name == name.as_str()) {
            return;
        }
        out.push(AliasEntry {
            name: Arc::from(name),
            target: target.clone(),
            source: AliasSource::Discovery,
        });
    };
    add(&kind.plural);
    add(&kind.singular);
    add(&kind.gvk.kind);
    for short in &kind.short_names {
        add(short);
    }
    if !kind.gvk.group.is_empty() {
        add(&format!("{}.{}", kind.plural, kind.gvk.group));
    }
}

#[cfg(test)]
mod tests {
    use oxikube_testkit::kinds::{KindSpec, kind};

    use super::*;

    #[test]
    fn version_rank_orders_ga_beta_alpha() {
        let mut versions = [
            "v1alpha1", "v2beta1", "v1", "v1beta2", "v2", "v1beta1", "junk",
        ];
        versions.sort_by_key(|v| std::cmp::Reverse(version_rank(v)));
        assert_eq!(
            versions,
            [
                "v2", "v1", "v2beta1", "v1beta2", "v1beta1", "v1alpha1", "junk"
            ]
        );
    }

    #[test]
    fn the_preferred_version_wins_whatever_the_order() {
        let beta = KindSpec::new("apps", "v1beta1", "Deployment", "deployments").build();
        let ga = KindSpec::new("apps", "v1", "Deployment", "deployments").build();
        let mut a = Discovered::default();
        a.replace_all(&[beta.clone(), ga.clone()]);
        let mut b = Discovered::default();
        b.replace_all(&[ga, beta]);
        assert_eq!(&*a.served_version("apps", "deployments").unwrap(), "v1");
        assert_eq!(&*b.served_version("apps", "deployments").unwrap(), "v1");
        assert_eq!(a.kind_count(), 1);
    }

    #[test]
    fn upsert_and_remove_touch_only_their_group() {
        let mut d = Discovered::default();
        d.replace_all(&[
            kind("", "v1", "Pod", "pods").short("po").build(),
            kind("example.io", "v1", "Widget", "widgets").build(),
        ]);
        let core: Vec<_> = d.entries().filter(|e| &*e.name == "po").collect();
        assert_eq!(core.len(), 1);

        d.upsert(&[kind("example.io", "v1", "Gadget", "gadgets").build()]);
        assert!(d.entries().any(|e| &*e.name == "gadgets.example.io"));
        assert!(d.entries().any(|e| &*e.name == "widgets"));

        d.remove(&[Gvk::new("example.io", "v1", "Widget")]);
        assert!(!d.entries().any(|e| &*e.name == "widgets"));
        assert!(d.entries().any(|e| &*e.name == "gadgets"));
        d.remove(&[Gvk::new("example.io", "v1", "Gadget")]);
        assert!(!d.groups.contains_key("example.io"), "an empty group goes");
    }

    #[test]
    fn a_type_contributes_each_word_once() {
        let mut d = Discovered::default();
        d.replace_all(&[kind("", "v1", "Node", "nodes")
            .short("no")
            .singular("node")
            .build()]);
        let mut names: Vec<_> = d.entries().map(|e| e.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["no", "node", "nodes"]);
    }
}
