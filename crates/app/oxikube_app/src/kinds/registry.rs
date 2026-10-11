//! [`KindRegistry`]: the registered kinds, looked up in O(1) by group and kind.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::command::{self, CommandId};
use oxikube_domain::ids::Gvk;

use super::gate::KindGate;
use super::meta::{GroupKind, KindMeta};
use super::tree::SidebarTree;
use crate::templates::{Template, TemplateCatalog};

/// Why a kind was not registered. Each is a programming error in a kind module.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterKindError {
    /// The kind is registered already (by another module, or twice).
    #[error("{0} is registered twice")]
    Duplicate(GroupKind),
    /// The kind lists a command the domain does not declare, so no palette, keymap or menu could
    /// ever run it.
    #[error("{kind} lists {action}, which is not a declared command")]
    UndeclaredAction {
        /// The kind.
        kind: GroupKind,
        /// The command id.
        action: CommandId,
    },
    /// The spec has no kind (only the built-in generic fallback may).
    #[error("a kind spec needs a kind and a plural")]
    Unnamed,
}

/// The registered kinds. Cheap to clone (the specs are shared).
///
/// Registration order never matters: lookups are by key, and the sidebar sorts by the sections'
/// and slots' ranks. [`kind_for`](Self::kind_for) answers every kind, falling back to the generic
/// spec, so custom resources and kinds from newer servers always open.
#[derive(Clone, Debug)]
pub struct KindRegistry {
    metas: Vec<Arc<KindMeta>>,
    index: HashMap<GroupKind, usize>,
    generic: Arc<KindMeta>,
}

impl Default for KindRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl KindRegistry {
    /// No kinds: every lookup answers the generic spec.
    pub fn new() -> Self {
        Self {
            metas: Vec::new(),
            index: HashMap::new(),
            generic: Arc::new(KindMeta::generic()),
        }
    }

    /// Registers `meta`.
    ///
    /// # Errors
    ///
    /// [`RegisterKindError`] when the kind is registered already, lists an undeclared command or
    /// is unnamed. Nothing is registered then.
    pub fn register(&mut self, meta: KindMeta) -> Result<(), RegisterKindError> {
        if meta.is_generic() || meta.plural.is_empty() {
            return Err(RegisterKindError::Unnamed);
        }
        let key = meta.group_kind();
        if self.index.contains_key(&key) {
            return Err(RegisterKindError::Duplicate(key));
        }
        if let Some(action) = meta
            .actions
            .iter()
            .copied()
            .find(|id| command::lookup(*id).is_none())
        {
            return Err(RegisterKindError::UndeclaredAction { kind: key, action });
        }
        self.index.insert(key, self.metas.len());
        self.metas.push(Arc::new(meta));
        Ok(())
    }

    /// The registered spec of `gvk` (any version), `None` when no module registered it.
    pub fn get(&self, gvk: &Gvk) -> Option<&Arc<KindMeta>> {
        self.index
            .get(&GroupKind::of(gvk))
            .map(|&ix| &self.metas[ix])
    }

    /// The spec of `gvk`: the registered one, else the [generic](KindMeta::generic) one. Never
    /// fails; the generic spec's own `gvk` is empty, so callers keep the `gvk` they asked with.
    pub fn kind_for(&self, gvk: &Gvk) -> &Arc<KindMeta> {
        self.get(gvk).unwrap_or(&self.generic)
    }

    /// Whether a module registered `gvk`.
    pub fn is_registered(&self, gvk: &Gvk) -> bool {
        self.index.contains_key(&GroupKind::of(gvk))
    }

    /// How many kinds are registered.
    pub fn len(&self) -> usize {
        self.metas.len()
    }

    /// Whether no kind is registered.
    pub fn is_empty(&self) -> bool {
        self.metas.is_empty()
    }

    /// The kinds that list `action` as their own.
    pub fn kinds_offering(&self, action: CommandId) -> impl Iterator<Item = &Arc<KindMeta>> {
        self.metas.iter().filter(move |meta| meta.offers(action))
    }

    /// The "Create resource" templates of `catalog` offered for `gvk` (see
    /// [`KindMeta::templates_in`]); none for an unregistered kind.
    pub fn templates_for<'a>(&self, gvk: &Gvk, catalog: &'a TemplateCatalog) -> Vec<&'a Template> {
        self.kind_for(gvk).templates_in(catalog)
    }

    /// The registered kinds that have a sidebar slot, by section, in rank order. With a `gate`,
    /// only those it [shows](KindGate::judge); without one, all of them. Built in one pass over
    /// the registry: cheap enough to rebuild on every discovery change.
    pub fn sidebar_tree(&self, gate: Option<&KindGate<'_>>) -> SidebarTree {
        SidebarTree::build(self.metas.iter(), gate)
    }
}
