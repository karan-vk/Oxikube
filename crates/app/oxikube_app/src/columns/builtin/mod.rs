//! [`CoreColumns`]: the [`ColumnProvider`] for core kinds read through reflector and metadata
//! feeds (ADR 0006), driven by the table-driven catalogue in `catalog`.
//!
//! The registry is built once: a map from kind name to the catalogue entries of that name
//! (several API groups can share one, such as `Event`). A lookup compares borrowed strings and
//! allocates nothing; `columns()` is memoised per `(group, kind, capabilities)`. Versions are
//! ignored, so `autoscaling/v1` and `autoscaling/v2` share a column set.

mod catalog;
mod def;
mod funcs;
mod metrics;
mod read;

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Capabilities, ObjectMeta, Resource};
use parking_lot::Mutex;

pub use self::def::Metric;
use self::def::{ColumnDef, GENERIC, KindDef, Src};
pub(super) use self::funcs::status_tone;
pub use self::metrics::MetricsSource;
pub(super) use self::read::scalar;
use super::{Cell, Column, ColumnId, ColumnProvider};
use crate::store::StoreObject;

type MemoKey = (Arc<str>, Arc<str>, Capabilities);

/// Column definitions and cells for the core kinds. See the [module docs](self).
pub struct CoreColumns {
    by_kind: HashMap<&'static str, Vec<&'static KindDef>>,
    memo: Mutex<HashMap<MemoKey, Arc<[Column]>>>,
    metrics: Option<Arc<dyn MetricsSource>>,
}

impl Default for CoreColumns {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CoreColumns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreColumns")
            .field("kinds", &self.by_kind.values().map(Vec::len).sum::<usize>())
            .field("metrics", &self.metrics.is_some())
            .finish_non_exhaustive()
    }
}

impl CoreColumns {
    /// The built-in catalogue, without a metrics source.
    pub fn new() -> Self {
        let mut by_kind: HashMap<&'static str, Vec<&'static KindDef>> = HashMap::new();
        for def in catalog::AREAS.iter().flat_map(|area| area.iter()) {
            by_kind.entry(def.kind).or_default().push(def);
        }
        Self {
            by_kind,
            memo: Mutex::new(HashMap::new()),
            metrics: None,
        }
    }

    /// Registers the source of CPU and memory cells (E13). Without one, those cells are
    /// [`Cell::Pending`].
    #[must_use]
    pub fn with_metrics(mut self, source: Arc<dyn MetricsSource>) -> Self {
        self.metrics = Some(source);
        self
    }

    /// Whether the catalogue has columns for `kind`; other kinds get the generic Name /
    /// Namespace / Age / Labels set.
    pub fn knows(&self, kind: &Gvk) -> bool {
        self.entry(&kind.group, &kind.kind).is_some()
    }

    /// Every `(group, kind)` the catalogue covers, in no particular order.
    pub fn kinds(&self) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        self.by_kind
            .values()
            .flatten()
            .map(|def| (def.group, def.kind))
    }

    fn entry(&self, group: &str, kind: &str) -> Option<&'static KindDef> {
        self.by_kind
            .get(kind)?
            .iter()
            .find(|def| def.group == group)
            .copied()
    }

    fn defs(&self, group: &str, kind: &str) -> &'static [ColumnDef] {
        self.entry(group, kind).map_or(GENERIC, |def| def.columns)
    }

    /// The cell of a [`Resource`] in `column`, as [`ColumnProvider::cell`] does for a
    /// [`StoreObject::Resource`].
    ///
    /// A metadata-only object ([`Resource::is_partial`]) has no `spec` or `status`, so only the
    /// metadata columns (name, namespace, age, labels) have values; computed columns stay blank
    /// rather than reading as `0/0` or `<none>`.
    pub fn resource_cell<'a>(
        &self,
        res: &'a Resource,
        column: &ColumnId,
        now: Timestamp,
    ) -> Cell<'a> {
        let defs = self.defs(&res.kind.group, &res.kind.kind);
        let Some(def) = defs.iter().find(|d| *column == *d.id) else {
            return Cell::empty();
        };
        match def.src {
            Src::Metric(metric) => self
                .metrics
                .as_ref()
                .and_then(|m| m.cell(res, metric, now))
                .unwrap_or(Cell::Pending),
            Src::Name | Src::Namespace | Src::Age | Src::Labels => read::read(def.src, res, now),
            _ if res.is_partial() => Cell::empty(),
            src => read::read(src, res, now),
        }
    }
}

impl ColumnProvider for CoreColumns {
    fn columns(&self, kind: &Gvk, caps: Capabilities) -> Arc<[Column]> {
        let key = (kind.group.clone(), kind.kind.clone(), caps);
        if let Some(hit) = self.memo.lock().get(&key) {
            return hit.clone();
        }
        let with_metrics = caps.contains(Capabilities::METRICS);
        let built: Arc<[Column]> = self
            .defs(&kind.group, &kind.kind)
            .iter()
            .filter(|d| with_metrics || !d.is_metric())
            .map(|d| Column {
                id: ColumnId::new(d.id),
                title: Arc::from(d.title),
                description: None,
                wide: d.wide,
                align: d.align,
                sort: d.sort,
                table_index: None,
            })
            .collect();
        self.memo.lock().insert(key, built.clone());
        built
    }

    fn cell<'a>(&self, object: &'a StoreObject, column: &ColumnId, now: Timestamp) -> Cell<'a> {
        match object {
            StoreObject::Resource(res) => self.resource_cell(res, column, now),
            StoreObject::Row(row) => meta_cell(&row.meta, column, now),
        }
    }
}

/// The columns every object has, read from its metadata alone: what a Table row, which carries
/// no spec, can answer.
pub(super) fn meta_cell<'a>(meta: &'a ObjectMeta, column: &ColumnId, now: Timestamp) -> Cell<'a> {
    match column.as_str() {
        ColumnId::NAME => read::name(meta),
        ColumnId::NAMESPACE => read::namespace(meta),
        ColumnId::AGE => read::age(meta, now),
        ColumnId::LABELS => read::labels(meta),
        _ => Cell::empty(),
    }
}
