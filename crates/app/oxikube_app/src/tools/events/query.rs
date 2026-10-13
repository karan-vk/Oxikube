//! Reading events out of the service: which object they are about, and a bounded, counted list.

use std::sync::Arc;

use oxikube_domain::AliasTarget;
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};

use super::render::object_label;
use crate::events::{EventFilter, EventService};
use crate::search::aliases::AliasRegistry;

/// An object by what an event says about it: the kind's name, the namespace (`None` for a
/// cluster-scoped object or when it is not restricted) and the object's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Involved {
    pub(super) kind: String,
    pub(super) namespace: Option<String>,
    pub(super) name: String,
}

impl Involved {
    pub(super) fn of(object: &ResourceRef) -> Self {
        Self {
            kind: object.gvk.kind.to_string(),
            namespace: object.namespace.as_deref().map(str::to_owned),
            name: object.name.to_string(),
        }
    }

    /// Whether `event` is about this object. Kinds compare case-insensitively (the filter's
    /// kind rule), names exactly.
    pub(super) fn matches(&self, event: &Event) -> bool {
        event.regarding.gvk.kind.eq_ignore_ascii_case(&self.kind)
            && *event.regarding.name == *self.name
            && self
                .namespace
                .as_deref()
                .is_none_or(|ns| event.regarding.namespace.as_deref() == Some(ns))
    }

    /// `Pod/shop/web-0`, or `Node/worker-1`.
    pub(super) fn label(&self) -> String {
        object_label(&self.kind, self.namespace.as_deref(), &self.name)
    }
}

/// What a read found: how many events matched, and the newest few of them.
#[derive(Debug, Default)]
pub(super) struct Found {
    /// Every match held.
    pub(super) total: usize,
    /// The newest matches, at most the limit, newest (by last seen) first.
    pub(super) events: Vec<Arc<Event>>,
}

impl Found {
    pub(super) fn truncated(&self) -> bool {
        self.total > self.events.len()
    }
}

/// The events of `cluster` that match `filter` and `keep`: the newest `limit` of them and the
/// count of all. One pass over the filter's narrowest index under the service's lock, copying
/// only reference counts.
pub(super) fn find(
    events: &EventService,
    cluster: &ClusterId,
    filter: &EventFilter,
    keep: impl Fn(&Event) -> bool,
    limit: usize,
) -> Found {
    events
        .read(cluster, |buffer| {
            let mut found = Found::default();
            for event in buffer.query(filter).filter(|event| keep(event)) {
                found.total += 1;
                if found.events.len() < limit {
                    found.events.push(event.clone());
                }
            }
            found
        })
        .unwrap_or_default()
}

/// [`find`] for the events about `object`.
pub(super) fn find_about(
    events: &EventService,
    cluster: &ClusterId,
    object: &Involved,
    limit: usize,
) -> Found {
    let filter = EventFilter::new().kinds([object.kind.as_str()]);
    find(
        events,
        cluster,
        &filter,
        |event| object.matches(event),
        limit,
    )
}

/// The type a kind word stands for in this cluster (`deploy` is `apps/v1 Deployment`), as its
/// alias table knows it; `None` for a word it does not know.
pub(super) fn resolve_gvk(aliases: &AliasRegistry, cluster: &ClusterId, word: &str) -> Option<Gvk> {
    let table = aliases.table(cluster);
    match table.resolve(word).target()? {
        AliasTarget::Gvr(gvr) => table.gvk_of(gvr),
        AliasTarget::Command { .. } => None,
    }
}

/// The Kind a word stands for in this cluster (`deploy` is `Deployment`); a word the cluster's
/// alias table does not know is taken as the Kind itself.
pub(super) fn kind_name(aliases: &AliasRegistry, cluster: &ClusterId, word: &str) -> String {
    resolve_gvk(aliases, cluster, word).map_or_else(|| word.to_owned(), |gvk| gvk.kind.to_string())
}
