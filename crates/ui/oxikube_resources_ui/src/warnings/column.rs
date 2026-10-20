//! The badge column and a table's source of badges.

use std::sync::Arc;

use gpui::App;
use jiff::Timestamp;
use oxikube_app::columns::{Align, SortKind};
use oxikube_app::store::StoreObject;
use oxikube_app::{
    ClusterBadges, Column, ColumnId, ObjectIdentity, ObjectTarget, WarningBadge, WarningBadges,
};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::ResourceKind;

use super::ObjectEvents;

/// The badge column's id (stable: it is the key of the saved layout).
pub const BADGE_COLUMN: &str = "warnings";

/// The badge column: narrow, an icon in the row's tone, not sortable (its cells read the live
/// events index, not the object), shown by default.
pub fn badge_column() -> Column {
    Column {
        id: ColumnId::new(BADGE_COLUMN),
        title: Arc::from("\u{26A0}"),
        description: Some(Arc::from(
            "Warning events about the object within the warning window (events.warning_window). \
             Click to see its events.",
        )),
        wide: false,
        align: Align::Left,
        sort: SortKind::Text,
        table_index: None,
        sortable: false,
    }
}

/// What a table reads its badges from: one cluster's badges, and how to find a row's object in
/// them.
#[derive(Clone)]
pub(crate) struct RowWarnings {
    badges: ClusterBadges,
    /// The kind the table lists: the events that gave no uid name their object by it.
    kind: Arc<str>,
    /// A cluster-scoped object is also looked up by name: the kubelet records a Node's events
    /// with the node's name as the uid.
    cluster_scoped: bool,
    handle: WarningBadges,
    /// When the table last checked for expired badges (the tick).
    checked: Timestamp,
}

impl RowWarnings {
    /// The source for a table of `kind` in `cluster`, when the app has an events index and the
    /// kind is one events are about (not Events themselves).
    pub(crate) fn for_table(cx: &App, cluster: &ClusterId, kind: &ResourceKind) -> Option<Self> {
        if kind.gvk.kind.as_ref() == "Event" {
            return None;
        }
        let handle = ObjectEvents::get(cx)?;
        Some(Self {
            badges: handle.for_cluster(cluster),
            kind: kind.gvk.kind.clone(),
            cluster_scoped: !kind.namespaced,
            handle,
            checked: Timestamp::now(),
        })
    }

    /// The handle, for the change stream.
    pub(crate) fn handle(&self) -> &WarningBadges {
        &self.handle
    }

    /// The warning that badges `object` at `now`, if any. `O(1)`.
    pub(crate) fn badge(&self, object: &StoreObject, now: Timestamp) -> Option<WarningBadge> {
        let meta = object.meta();
        // The events whose writer gave no uid name their object by kind, namespace and name.
        // The parts are shared: a row reads its badge on every frame.
        let target = ObjectTarget::named(
            meta.uid.clone(),
            ObjectIdentity::shared(self.kind.clone(), meta.namespace.clone(), meta.name.clone()),
        );
        self.badges
            .get_for(&target, self.cluster_scoped.then_some(&*meta.name), now)
    }

    /// Whether a badge expired since the last call (the tick redraws then): some warning was
    /// badged at the last check and is not at `now`. Looks at the crossing, not at a count of
    /// stale entries, so an expiry cannot hide behind a replacement or an eviction.
    pub(crate) fn expired_moved(&mut self, now: Timestamp) -> bool {
        if now <= self.checked {
            return false;
        }
        let from = std::mem::replace(&mut self.checked, now);
        self.badges.expired_between(from, now) > 0
    }
}
