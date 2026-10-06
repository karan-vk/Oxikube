//! [`StoreQuery`]: what a subscriber wants (kind, scope, in-app filter and sort), and the
//! [`SortValue`] each object is ranked by.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::session::WatchScope;
use serde_json::Value;

use super::object::{FeedScope, StoreObject};
use super::selector::LabelSelector;
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
}

impl StoreQuery {
    /// Every object of `gvk` in `scope`, in kubectl order (namespace, then name).
    pub fn new(gvk: Gvk, scope: WatchScope) -> Self {
        Self {
            gvk,
            scope,
            filter: StoreFilter::default(),
            sort: SortKey::default(),
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

    /// The feeds this query reads, one per [`FeedScope`], in order.
    pub(crate) fn parts(&self) -> Vec<FeedScope> {
        parts_of(&self.scope)
    }
}

/// The [`FeedScope`]s a [`WatchScope`] is served by.
pub(crate) fn parts_of(scope: &WatchScope) -> Vec<FeedScope> {
    match scope {
        WatchScope::Cluster => vec![FeedScope::Cluster],
        WatchScope::Namespaces(names) => names
            .iter()
            .map(|n| FeedScope::Namespace(Arc::from(n.as_str())))
            .collect(),
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
    }
}

/// ASCII-case-insensitive substring search without allocating.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// What to sort by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SortField {
    /// Namespace, then name (kubectl's order). The default.
    #[default]
    Namespace,
    /// Name, then namespace.
    Name,
    /// `metadata.creationTimestamp` (oldest first when ascending).
    Created,
    /// The value of one label (objects without it last).
    Label(String),
    /// One Table cell, by column index (objects without cells last).
    Column(usize),
}

/// A sort field and direction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortKey {
    /// The field.
    pub field: SortField,
    /// Reverse the order.
    pub descending: bool,
}

impl SortKey {
    /// Ascending by `field`.
    pub fn by(field: SortField) -> Self {
        Self {
            field,
            descending: false,
        }
    }

    /// The same field, descending.
    #[must_use]
    pub fn descending(mut self) -> Self {
        self.descending = true;
        self
    }

    /// The value `object` is ranked by under this key. Ties break on the object key.
    pub(crate) fn value_of(&self, object: &StoreObject) -> SortValue {
        let meta = object.meta();
        match &self.field {
            SortField::Namespace => SortValue::Missing,
            SortField::Name => SortValue::Text(meta.name.clone()),
            SortField::Created => meta.creation.map_or(SortValue::Missing, SortValue::Time),
            SortField::Label(key) => meta
                .labels
                .get(key.as_str())
                .map_or(SortValue::Missing, |v| SortValue::Text(v.clone())),
            SortField::Column(i) => object
                .cells()
                .and_then(|cells| cells.get(*i))
                .map_or(SortValue::Missing, cell_value),
        }
    }
}

fn cell_value(cell: &Value) -> SortValue {
    match cell {
        Value::Number(n) => n
            .as_i64()
            .map(SortValue::Int)
            .or_else(|| n.as_f64().map(SortValue::Float))
            .unwrap_or(SortValue::Missing),
        Value::String(s) => SortValue::Text(Arc::from(s.as_str())),
        Value::Bool(b) => SortValue::Int(i64::from(*b)),
        Value::Null | Value::Array(_) | Value::Object(_) => SortValue::Missing,
    }
}

/// The value an object is ranked by. Numbers sort before text, text before times, and a
/// missing value after everything (so blanks sink to the bottom when ascending).
#[derive(Debug, Clone)]
pub(crate) enum SortValue {
    Int(i64),
    Float(f64),
    Text(Arc<str>),
    Time(Timestamp),
    Missing,
}

impl SortValue {
    fn rank(&self) -> u8 {
        match self {
            SortValue::Int(_) | SortValue::Float(_) => 0,
            SortValue::Text(_) => 1,
            SortValue::Time(_) => 2,
            SortValue::Missing => 3,
        }
    }
}

impl Ord for SortValue {
    fn cmp(&self, other: &Self) -> Ordering {
        use SortValue::{Float, Int, Text, Time};
        match (self, other) {
            (Int(a), Int(b)) => a.cmp(b),
            #[allow(
                clippy::cast_precision_loss,
                reason = "mixed int/float cells compare as f64"
            )]
            (Int(a), Float(b)) => (*a as f64).total_cmp(b),
            #[allow(
                clippy::cast_precision_loss,
                reason = "mixed int/float cells compare as f64"
            )]
            (Float(a), Int(b)) => a.total_cmp(&(*b as f64)),
            (Float(a), Float(b)) => a.total_cmp(b),
            (Text(a), Text(b)) => a.cmp(b),
            (Time(a), Time(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for SortValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SortValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortValue {}

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
    fn sort_values_order_numbers_text_time_then_missing() {
        let mut values = [
            SortValue::Missing,
            SortValue::Text("b".into()),
            SortValue::Float(1.5),
            SortValue::Int(2),
            SortValue::Text("a".into()),
            SortValue::Int(1),
        ];
        values.sort();
        assert!(matches!(values[0], SortValue::Int(1)));
        assert!(matches!(values[1], SortValue::Float(_)));
        assert!(matches!(values[2], SortValue::Int(2)));
        assert!(matches!(&values[3], SortValue::Text(t) if &**t == "a"));
        assert!(matches!(values[5], SortValue::Missing));
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
        assert_eq!(parts_of(&WatchScope::Cluster), vec![FeedScope::Cluster]);
    }
}
