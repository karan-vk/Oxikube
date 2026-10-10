//! The lookup index: every layer merged into one hash map by lower-case name, plus the conflicts
//! found while merging.
//!
//! Precedence is `User > BuiltIn > Discovery`. A name held by a higher layer resolves there; a
//! lower layer's entries for the same name that lead somewhere else are recorded as
//! [`ConflictKind::Shadowed`]. Within the discovery layer, several destinations for one name are
//! [`ConflictKind::Ambiguous`] and sorted core group first, then by group and plural, so the
//! first candidate never depends on the order discovery listed things in.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::AliasTarget;

use super::entry::{AliasConflict, AliasEntry, AliasSource, ConflictKind, Resolution};

/// What a name resolves to.
#[derive(Debug)]
enum Slot {
    One(AliasEntry),
    Many(Arc<[AliasEntry]>),
}

impl Slot {
    /// The entry that decides the name (the first candidate of an ambiguous one).
    fn winner(&self) -> &AliasEntry {
        match self {
            Self::One(entry) => entry,
            Self::Many(candidates) => &candidates[0],
        }
    }
}

/// The merged table. Immutable once built; the table swaps whole indexes.
#[derive(Debug, Default)]
pub(super) struct Index {
    map: HashMap<Arc<str>, Slot>,
    conflicts: Vec<AliasConflict>,
}

/// The entries of every layer that claim one name.
#[derive(Default)]
struct Claims {
    user: Option<AliasEntry>,
    builtin: Option<AliasEntry>,
    discovery: Vec<AliasEntry>,
}

impl Index {
    /// Merges the layers.
    pub(super) fn build<'a>(
        user: &[AliasEntry],
        builtin: &[AliasEntry],
        discovery: impl Iterator<Item = &'a AliasEntry>,
    ) -> Self {
        let mut claims: HashMap<Arc<str>, Claims> =
            HashMap::with_capacity(builtin.len() + user.len());
        for entry in user {
            claims.entry(entry.name.clone()).or_default().user = Some(entry.clone());
        }
        for entry in builtin {
            claims.entry(entry.name.clone()).or_default().builtin = Some(entry.clone());
        }
        for entry in discovery {
            claims
                .entry(entry.name.clone())
                .or_default()
                .discovery
                .push(entry.clone());
        }

        let mut index = Self {
            map: HashMap::with_capacity(claims.len()),
            conflicts: Vec::new(),
        };
        for (name, claim) in claims {
            index.settle(name, claim);
        }
        index
            .conflicts
            .sort_by(|a, b| (&a.name, a.kind as u8).cmp(&(&b.name, b.kind as u8)));
        index
    }

    fn settle(&mut self, name: Arc<str>, claims: Claims) {
        let Claims {
            user,
            builtin,
            mut discovery,
        } = claims;
        sort_candidates(&mut discovery);

        // The highest layer that has the name decides.
        let (winner, lower): (AliasEntry, Vec<AliasEntry>) = match (user, builtin) {
            (Some(user), builtin) => (user, builtin.into_iter().chain(discovery).collect()),
            (None, Some(builtin)) => (builtin, discovery),
            (None, None) => {
                self.settle_discovery(name, discovery);
                return;
            }
        };
        let others: Vec<AliasEntry> = lower
            .into_iter()
            .filter(|entry| !entry.target.same_destination(&winner.target))
            .collect();
        if !others.is_empty() {
            self.conflicts.push(AliasConflict {
                name: name.clone(),
                kind: ConflictKind::Shadowed,
                winner: winner.clone(),
                others,
            });
        }
        self.map.insert(name, Slot::One(winner));
    }

    fn settle_discovery(&mut self, name: Arc<str>, mut candidates: Vec<AliasEntry>) {
        match candidates.len() {
            0 => {}
            1 => {
                self.map.insert(name, Slot::One(candidates.remove(0)));
            }
            _ => {
                self.conflicts.push(AliasConflict {
                    name: name.clone(),
                    kind: ConflictKind::Ambiguous,
                    winner: candidates[0].clone(),
                    others: candidates[1..].to_vec(),
                });
                self.map.insert(name, Slot::Many(candidates.into()));
            }
        }
    }

    /// The entry or entries called `name` (already lower-case).
    pub(super) fn lookup(&self, name: &str) -> Option<Resolution> {
        self.map.get(name).map(|slot| match slot {
            Slot::One(entry) => Resolution::Exact(entry.clone()),
            Slot::Many(candidates) => Resolution::Ambiguous(candidates.clone()),
        })
    }

    /// Every name with its winning entry (the first candidate of an ambiguous name), by name.
    pub(super) fn entries(&self) -> Vec<AliasEntry> {
        let mut all: Vec<AliasEntry> = self
            .map
            .values()
            .map(|slot| slot.winner().clone())
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        all
    }

    pub(super) fn conflicts(&self) -> &[AliasConflict] {
        &self.conflicts
    }

    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    /// The names, for suggestions.
    pub(super) fn names(&self) -> impl Iterator<Item = &Arc<str>> {
        self.map.keys()
    }

    /// The source of the entry called `name`, for ordering suggestions.
    pub(super) fn source_of(&self, name: &str) -> Option<AliasSource> {
        self.map.get(name).map(|slot| slot.winner().source)
    }
}

/// Fixed order for discovery candidates: core group first, then by group, resource and version.
/// Entries leading to the same type (a singular that equals the Kind) are merged.
fn sort_candidates(candidates: &mut Vec<AliasEntry>) {
    fn key(entry: &AliasEntry) -> (&str, &str, &str) {
        match &entry.target {
            AliasTarget::Gvr(gvr) => (&gvr.group, &gvr.resource, &gvr.version),
            AliasTarget::Command { name, .. } => ("\u{10ffff}", name, ""),
        }
    }
    candidates.sort_by(|a, b| key(a).cmp(&key(b)));
    candidates.dedup_by(|b, a| a.target.same_destination(&b.target));
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::Gvr;

    use super::*;

    fn entry(name: &str, group: &str, resource: &str, source: AliasSource) -> AliasEntry {
        AliasEntry {
            name: Arc::from(name),
            target: AliasTarget::Gvr(Gvr::new(group, "v1", resource)),
            source,
        }
    }

    #[test]
    fn user_beats_builtin_beats_discovery_and_the_losers_are_listed() {
        let user = [entry("x", "u.io", "users", AliasSource::User)];
        let builtin = [
            entry("x", "", "pods", AliasSource::BuiltIn),
            entry("y", "", "pods", AliasSource::BuiltIn),
        ];
        let discovery = [
            entry("x", "d.io", "xs", AliasSource::Discovery),
            entry("y", "", "pods", AliasSource::Discovery),
        ];
        let index = Index::build(&user, &builtin, discovery.iter());

        let Some(Resolution::Exact(x)) = index.lookup("x") else {
            panic!("x is exact");
        };
        assert_eq!(x.source, AliasSource::User);
        let Some(Resolution::Exact(y)) = index.lookup("y") else {
            panic!("y is exact");
        };
        assert_eq!(y.source, AliasSource::BuiltIn);

        // `y` agrees with discovery, so it is no conflict; `x` has two losers.
        let conflicts = index.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(&*conflicts[0].name, "x");
        assert_eq!(conflicts[0].kind, ConflictKind::Shadowed);
        assert_eq!(conflicts[0].others.len(), 2);
    }

    #[test]
    fn two_groups_with_one_plural_are_ambiguous_core_group_first() {
        let discovery = [
            entry("certs", "z.io", "certs", AliasSource::Discovery),
            entry("certs", "a.io", "certs", AliasSource::Discovery),
            entry("certs", "", "certs", AliasSource::Discovery),
        ];
        let index = Index::build(&[], &[], discovery.iter());
        let Some(Resolution::Ambiguous(candidates)) = index.lookup("certs") else {
            panic!("certs is ambiguous");
        };
        let groups: Vec<_> = candidates
            .iter()
            .map(|e| match &e.target {
                AliasTarget::Gvr(g) => g.group.to_string(),
                AliasTarget::Command { .. } => unreachable!(),
            })
            .collect();
        assert_eq!(groups, ["", "a.io", "z.io"]);
        assert_eq!(index.conflicts()[0].kind, ConflictKind::Ambiguous);
        assert_eq!(index.conflicts()[0].others.len(), 2);
    }
}
