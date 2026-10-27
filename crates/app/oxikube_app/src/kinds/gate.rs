//! Which kinds a cluster's sidebar shows: what discovery serves ([`ServedKinds`]) and what the
//! visibility hook allows ([`KindVisibility`]), joined by [`KindGate`].

use std::collections::HashMap;

use futures::future::BoxFuture;

use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::session::NamespaceSelection;

use super::meta::GroupKind;

/// The kinds a cluster serves, from discovery (any version). Optional kinds (Gateway API,
/// VolumeSnapshots, VPA) appear only on clusters that have them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServedKinds {
    /// Each served kind with its plural resource name (what RBAC rules name it by).
    kinds: HashMap<GroupKind, String>,
}

impl ServedKinds {
    /// The kinds of a discovery answer.
    pub fn from_kinds<'a>(kinds: impl IntoIterator<Item = &'a ResourceKind>) -> Self {
        Self {
            kinds: kinds
                .into_iter()
                .map(|k| (GroupKind::of(&k.gvk), k.plural.clone()))
                .collect(),
        }
    }

    /// The plural resource name `gvk` is served as (`None` when it is not served).
    pub fn plural(&self, gvk: &Gvk) -> Option<&str> {
        self.kinds.get(&GroupKind::of(gvk)).map(String::as_str)
    }

    /// Whether the cluster serves `gvk` at some version.
    pub fn serves(&self, gvk: &Gvk) -> bool {
        self.kinds.contains_key(&GroupKind::of(gvk))
    }

    /// How many kinds the cluster serves.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// Whether discovery listed nothing.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

/// Whether the user may list a kind: the hook the sidebar asks before showing an entry.
///
/// E17 implements it over `SelfSubjectRulesReview` (the `AccessService`); until then the default
/// allows everything. The sidebar's own rules review (E06-S10) still hides sections the user may
/// not list, so this hook is for finer, per-kind gating. Implementations answer from a cache
/// (this runs while the sidebar rebuilds, on the UI thread): no RBAC call here.
pub trait KindVisibility: Send + Sync {
    /// Whether the user may list `gvk` in `cluster`, in `namespace` (`None`: cluster-wide, or a
    /// cluster-scoped kind). `None` when not known yet, which shows the kind: only a definite
    /// `Some(false)` hides it.
    fn is_listable(&self, cluster: &ClusterId, gvk: &Gvk, namespace: Option<&str>) -> Option<bool> {
        let _ = (cluster, gvk, namespace);
        None
    }

    /// [`is_listable`](Self::is_listable) for an implementation that asks RBAC, which names a
    /// kind by its plural resource name (`plural`, from discovery; `None` when discovery has not
    /// answered). The gate calls this one; it defaults to [`is_listable`](Self::is_listable).
    fn is_listable_resource(
        &self,
        cluster: &ClusterId,
        gvk: &Gvk,
        plural: Option<&str>,
        namespace: Option<&str>,
    ) -> Option<bool> {
        let _ = plural;
        self.is_listable(cluster, gvk, namespace)
    }

    /// Loads what [`is_listable`](Self::is_listable) answers from, for `cluster` and the
    /// selected `namespaces`: the sidebar awaits it (off the UI thread) after connect and on every
    /// namespace change, then rebuilds. The default has nothing to load.
    fn refresh<'a>(
        &'a self,
        cluster: &'a ClusterId,
        namespaces: &'a NamespaceSelection,
    ) -> BoxFuture<'a, ()> {
        let _ = (cluster, namespaces);
        Box::pin(std::future::ready(()))
    }
}

/// The default [`KindVisibility`]: allows every kind.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAll;

impl KindVisibility for AllowAll {}

/// What a [`KindGate`] says about one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// The entry is shown.
    Yes,
    /// The cluster does not serve the kind: not an access matter, nothing to explain.
    NotServed,
    /// The visibility hook says the user may not list it.
    NotListable,
}

/// The rule that decides whether a cluster's sidebar shows a kind. Borrowed, built per rebuild.
#[derive(Clone, Copy)]
pub struct KindGate<'a> {
    /// The cluster the sidebar belongs to.
    pub cluster: &'a ClusterId,
    /// What discovery serves; `None` until it answers (or when it failed): everything counts as
    /// served, as a review that has not answered hides nothing.
    pub served: Option<&'a ServedKinds>,
    /// The visibility hook.
    pub visibility: &'a dyn KindVisibility,
    /// The selected namespaces: a namespaced kind is listable when it is in any of them.
    pub namespaces: &'a NamespaceSelection,
}

impl KindGate<'_> {
    /// Whether the sidebar shows `gvk` (of `scope`).
    pub fn judge(&self, gvk: &Gvk, scope: Scope) -> Shown {
        if self.served.is_some_and(|served| !served.serves(gvk)) {
            return Shown::NotServed;
        }
        let plural = self.served.and_then(|served| served.plural(gvk));
        let listable = |namespace: Option<&str>| {
            self.visibility
                .is_listable_resource(self.cluster, gvk, plural, namespace)
                .unwrap_or(true)
        };
        let allowed = if scope.is_namespaced() && !self.namespaces.is_all() {
            self.namespaces.names().any(|ns| listable(Some(ns)))
        } else {
            listable(None)
        };
        if allowed {
            Shown::Yes
        } else {
            Shown::NotListable
        }
    }
}
