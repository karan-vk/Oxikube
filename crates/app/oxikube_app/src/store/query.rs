//! [`StoreQuery`]: what a subscriber wants (kind, scope, in-app filter and sort).

use std::collections::BTreeSet;
use std::sync::Arc;

use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::session::WatchScope;

use super::filter::NameFilter;
use super::object::{FeedScope, StoreObject};
use super::selector::LabelSelector;
use super::sort::SortKey;
use crate::session::ClusterSession;

/// One subscription's question to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreQuery {
    /// The kind.
    pub gvk: Gvk,
    /// Which part of the cluster to watch.
    pub scope: WatchScope,
    /// In-app filter applied to the cached objects.
    pub filter: StoreFilter,
    /// In-app sort order.
    pub sort: SortKey,
    /// Label selector the server applies (`/-l`): the feeds this query reads are the ones keyed
    /// with it, so the API returns only the matches. `None` reads every object. Unlike
    /// [`StoreFilter::labels`], which filters cached objects, this changes what is fetched.
    pub selector: Option<LabelSelector>,
}

impl StoreQuery {
    /// Every object of `gvk` in `scope`, in kubectl order (namespace, then name).
    pub fn new(gvk: Gvk, scope: WatchScope) -> Self {
        Self {
            gvk,
            scope,
            filter: StoreFilter::default(),
            sort: SortKey::default(),
            selector: None,
        }
    }

    /// `gvk` under `session`'s current namespace selection; `kind_scope` says whether the kind
    /// is namespaced (from discovery).
    pub fn for_session(gvk: Gvk, kind_scope: Scope, session: &ClusterSession) -> Self {
        Self::new(gvk, session.watch_scope(kind_scope))
    }

    /// Sets the filter.
    #[must_use]
    pub fn with_filter(mut self, filter: StoreFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Sets the sort order.
    #[must_use]
    pub fn with_sort(mut self, sort: SortKey) -> Self {
        self.sort = sort;
        self
    }

    /// Sets the server-side label selector (an empty one reads everything).
    #[must_use]
    pub fn with_selector(mut self, selector: Option<LabelSelector>) -> Self {
        self.selector = selector.filter(|s| !s.is_empty());
        self
    }

    /// The feeds this query reads, one per [`FeedScope`], in order.
    pub(crate) fn parts(&self) -> Vec<FeedScope> {
        match &self.scope {
            WatchScope::Cluster => vec![FeedScope::Cluster],
            WatchScope::Namespaces(names) => names
                .iter()
                .map(|n| FeedScope::Namespace(Arc::from(n.as_str())))
                .collect(),
        }
    }
}

/// An in-app filter over cached objects. Every set field must match (AND); the default
/// matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreFilter {
    /// Case-insensitive substring of the name (the `/` filter).
    pub text: Option<String>,
    /// Exact name (answered from the name index).
    pub name: Option<String>,
    /// Only these namespaces (answered from the namespace index).
    pub namespaces: Option<BTreeSet<String>>,
    /// Label selector (`-l`; equality terms answered from the label index).
    pub labels: Option<LabelSelector>,
    /// The name pattern of the `/` filter: text or regex, inverse, or fuzzy (E07-S04).
    pub pattern: Option<NameFilter>,
}

impl StoreFilter {
    /// A filter on a name substring.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::default()
        }
    }

    /// A filter on a label selector.
    pub fn labels(selector: LabelSelector) -> Self {
        Self {
            labels: Some(selector),
            ..Self::default()
        }
    }

    /// Whether this filter lets everything through.
    pub fn is_empty(&self) -> bool {
        self.text.as_deref().is_none_or(str::is_empty)
            && self.name.is_none()
            && self.namespaces.is_none()
            && self.labels.as_ref().is_none_or(LabelSelector::is_empty)
            && self.pattern.as_ref().is_none_or(NameFilter::is_empty)
    }

    /// Whether every object this filter passes is also passed by `older`, so applying it to
    /// `older`'s rows gives the rows a full pass over the cache would (the table's "typed one
    /// more character" case). Conservative: `false` means "recompute from the cache".
    pub fn narrows(&self, older: &StoreFilter) -> bool {
        let same_fields = self.text == older.text
            && self.name == older.name
            && self.namespaces == older.namespaces
            && self.labels == older.labels;
        same_fields
            && match (&self.pattern, &older.pattern) {
                (_, None) => true,
                (None, Some(old)) => old.is_empty(),
                (Some(new), Some(old)) => new == old || new.narrows(old),
            }
    }

    /// Whether `object` passes.
    pub fn matches(&self, object: &StoreObject) -> bool {
        let meta = object.meta();
        self.name.as_deref().is_none_or(|n| *meta.name == *n)
            && self
                .text
                .as_deref()
                .is_none_or(|t| contains_ignore_case(&meta.name, t))
            && self
                .namespaces
                .as_ref()
                .is_none_or(|set| meta.namespace.as_deref().is_some_and(|ns| set.contains(ns)))
            && self.labels.as_ref().is_none_or(|s| s.matches(&meta.labels))
            && self.pattern.as_ref().is_none_or(|p| p.matches(&meta.name))
    }
}

/// ASCII-case-insensitive substring search without allocating.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_testkit::pod;

    fn obj(name: &str) -> StoreObject {
        StoreObject::Resource(pod().name(name).label("app", "web").build())
    }

    #[test]
    fn filter_fields_are_anded() {
        let o = obj("Web-0");
        assert!(StoreFilter::default().matches(&o));
        assert!(StoreFilter::default().is_empty());
        assert!(StoreFilter::text("web").matches(&o), "case-insensitive");
        assert!(!StoreFilter::text("db").matches(&o));
        let labels = StoreFilter::labels(LabelSelector::parse("app=web").unwrap());
        assert!(labels.matches(&o));
        let both = StoreFilter {
            text: Some("web".into()),
            labels: Some(LabelSelector::parse("app=db").unwrap()),
            ..StoreFilter::default()
        };
        assert!(!both.matches(&o));
        let ns = StoreFilter {
            namespaces: Some(BTreeSet::from(["elsewhere".to_owned()])),
            ..StoreFilter::default()
        };
        assert!(!ns.matches(&o));
    }

    #[test]
    fn parts_split_namespaces_into_one_feed_each() {
        let q = StoreQuery::new(
            Gvk::new("", "v1", "Pod"),
            WatchScope::Namespaces(vec!["a".into(), "b".into()]),
        );
        assert_eq!(
            q.parts(),
            vec![
                FeedScope::Namespace("a".into()),
                FeedScope::Namespace("b".into())
            ]
        );
        assert_eq!(
            StoreQuery::new(Gvk::new("", "v1", "Pod"), WatchScope::Cluster).parts(),
            vec![FeedScope::Cluster]
        );
    }
}
