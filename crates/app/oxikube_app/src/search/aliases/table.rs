//! [`AliasTable`]: the names one cluster's jump bar understands.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::{Gvk, Gvr};
use oxikube_domain::kinds::ResourceKind;
use parking_lot::{Mutex, RwLock};

use super::builtin;
use super::discovered::Discovered;
use super::entry::{AliasConflict, AliasEntry, AliasSource, Resolution};
use super::index::Index;
use super::suggest::suggest;

/// The alias table of one cluster: user aliases, then the built-in k9s names, then what the
/// cluster's discovery served.
///
/// A handle: clones share the table. Reads ([`resolve`](Self::resolve)) take a read lock for one
/// hash lookup and allocate nothing for a known name; every change rebuilds the merged index
/// outside that lock and swaps it in, so a jump-bar keystroke never waits for a rebuild. Changes
/// are meant for a background task (the registry's follower, the `aliases.json` watcher), never
/// the UI thread.
#[derive(Clone)]
pub struct AliasTable {
    inner: Arc<Inner>,
}

struct Inner {
    /// What the index is built from. Held for the whole of a change, so changes are serial.
    inputs: Mutex<Inputs>,
    /// The merged lookup.
    index: RwLock<Arc<Index>>,
}

#[derive(Default)]
struct Inputs {
    discovered: Discovered,
    user: Vec<AliasEntry>,
}

impl Default for AliasTable {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for AliasTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AliasTable")
            .field("names", &self.inner.index.read().len())
            .finish_non_exhaustive()
    }
}

impl AliasTable {
    /// A table with the built-in aliases only (a cluster that has not answered discovery yet).
    pub fn new() -> Self {
        let inputs = Inputs::default();
        let index = build(&inputs);
        Self {
            inner: Arc::new(Inner {
                inputs: Mutex::new(inputs),
                index: RwLock::new(Arc::new(index)),
            }),
        }
    }

    /// What `name` stands for, ignoring case and surrounding blanks.
    ///
    /// O(1) and allocation-free for a known name (the entry is a handful of reference-count
    /// bumps); an unknown name also scans for [`Resolution::Unknown`] suggestions.
    pub fn resolve(&self, name: &str) -> Resolution {
        let index = self.inner.index.read().clone();
        with_lowercase(name.trim(), |key| {
            index.lookup(key).unwrap_or_else(|| Resolution::Unknown {
                suggestions: suggest(&index, key),
            })
        })
    }

    /// The `Gvk` to open the list of `gvr` by: the Kind of that type, as the cluster serves it
    /// (or, before it has answered, as the built-in table knows it), at `gvr`'s version. `None`
    /// for a type nobody knows the Kind of (a user alias to a resource the cluster does not
    /// serve). O(1).
    pub fn gvk_of(&self, gvr: &Gvr) -> Option<Gvk> {
        let kind = self.inner.index.read().kind_of(&gvr.group, &gvr.resource)?;
        Some(Gvk::new(gvr.group.clone(), gvr.version.clone(), kind))
    }

    /// Replaces the discovery layer with everything the cluster serves.
    pub fn set_discovered(&self, kinds: &[ResourceKind]) {
        self.change(|inputs| inputs.discovered.replace_all(kinds));
    }

    /// Applies a CRD change: forgets the types of `removed` (by group and Kind) and adds or
    /// replaces `upserted`. Only the groups involved are recomputed.
    pub fn apply_kinds_change(&self, removed: &[Gvk], upserted: &[ResourceKind]) {
        self.change(|inputs| {
            inputs.discovered.remove(removed);
            inputs.discovered.upsert(upserted);
        });
    }

    /// Forgets the discovery layer (the cluster disconnected); built-in and user aliases stay.
    pub fn clear_discovered(&self) {
        self.change(|inputs| inputs.discovered.clear());
    }

    /// Replaces the user layer with `aliases` (name and target). Names are lower-cased; a blank
    /// name or one with whitespace is skipped, and the last of two equal names wins.
    pub fn set_user_aliases(&self, aliases: impl IntoIterator<Item = (String, AliasTarget)>) {
        let mut by_name: HashMap<Arc<str>, AliasEntry> = HashMap::new();
        for (name, target) in aliases {
            let name = name.trim().to_ascii_lowercase();
            if name.is_empty() || name.contains(char::is_whitespace) {
                continue;
            }
            let name: Arc<str> = Arc::from(name);
            by_name.insert(
                name.clone(),
                AliasEntry {
                    name,
                    target,
                    source: AliasSource::User,
                },
            );
        }
        let mut user: Vec<AliasEntry> = by_name.into_values().collect();
        user.sort_by(|a, b| a.name.cmp(&b.name));
        self.change(|inputs| inputs.user = user);
    }

    /// The names that more than one entry claims with different destinations, and how each was
    /// settled, by name. For the help overlay (E11-S10); never empty by accident: a shadowed
    /// built-in or an ambiguous CRD name is always here.
    pub fn conflicts(&self) -> Vec<AliasConflict> {
        self.inner.index.read().conflicts().to_vec()
    }

    /// Every known name with the entry it resolves to (the first candidate of an ambiguous
    /// name), by name. For an alias browser (k9s's `ctrl-a`).
    pub fn entries(&self) -> Vec<AliasEntry> {
        self.inner.index.read().entries()
    }

    /// How many names the table knows.
    pub fn len(&self) -> usize {
        self.inner.index.read().len()
    }

    /// Whether the table knows no name (never true: the built-ins are always there).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn change(&self, apply: impl FnOnce(&mut Inputs)) {
        let mut inputs = self.inner.inputs.lock();
        apply(&mut inputs);
        let index = Arc::new(build(&inputs));
        *self.inner.index.write() = index;
    }
}

/// Merges the layers of `inputs`.
fn build(inputs: &Inputs) -> Index {
    let builtin = builtin::entries(&inputs.discovered);
    let mut kinds: HashMap<(Arc<str>, Arc<str>), Arc<str>> = builtin::BUILTINS
        .iter()
        .map(|row| ((row.group.into(), row.resource.into()), row.kind.into()))
        .collect();
    for kind in inputs.discovered.kinds() {
        kinds.insert(
            (kind.gvk.group.clone(), Arc::from(kind.plural.as_str())),
            kind.gvk.kind.clone(),
        );
    }
    Index::build(&inputs.user, &builtin, inputs.discovered.entries()).with_kinds(kinds)
}

/// Calls `f` with `name` in lower case, without allocating for the usual short names.
fn with_lowercase<R>(name: &str, f: impl FnOnce(&str) -> R) -> R {
    const STACK: usize = 96;
    if !name.bytes().any(|b| b.is_ascii_uppercase()) {
        return f(name);
    }
    if name.len() <= STACK {
        let mut buf = [0u8; STACK];
        let bytes = &mut buf[..name.len()];
        bytes.copy_from_slice(name.as_bytes());
        bytes.make_ascii_lowercase();
        // Lower-casing ASCII letters keeps valid UTF-8 valid.
        if let Ok(lower) = std::str::from_utf8(bytes) {
            return f(lower);
        }
    }
    f(&name.to_ascii_lowercase())
}
