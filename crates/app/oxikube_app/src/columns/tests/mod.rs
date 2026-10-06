//! Tests for [`super`]: fixtures in, expected columns and cell texts out.
//!
//! * `pods`: the pod status, ready and restarts rules, with the fixtures and builders.
//! * `kinds`: one table-driven case per kind (fixture JSON to cell texts) for the ~40 kinds.
//! * `registry`: invariants of the catalogue (ids, wide order, feed policy, metrics, memo).
//! * `sorting`: sort keys compare by value, not text.
//! * `table`: the Table provider on recorded server responses.

mod kinds;
mod kinds_more;
mod pods;
mod registry;
mod sorting;
mod table;

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_testkit::builders::ResourceBuilder;
use oxikube_testkit::resource;
use serde_json::Value;

use super::{Cell, ColumnId, CoreColumns};

/// The reference "now": 27 hours 4 minutes after the fixtures' creation time
/// (`2026-01-01T00:00:00Z`), so ages print as `27h`.
pub(super) fn now() -> Timestamp {
    "2026-01-02T03:04:05Z".parse().unwrap()
}

/// The core provider's cell of `res` in column `id`.
pub(super) fn cell<'a>(res: &'a Resource, id: &str) -> Cell<'a> {
    CoreColumns::new().resource_cell(res, &ColumnId::new(id), now())
}

/// The display text of `res` in column `id`.
pub(super) fn text(res: &Resource, id: &str) -> String {
    cell(res, id).display().to_owned()
}

/// Asserts the display text of every `(column id, text)` pair, naming the object on failure.
#[track_caller]
pub(super) fn check(res: &Resource, expected: &[(&str, &str)]) {
    let provider = CoreColumns::new();
    for (id, want) in expected {
        let got = provider.resource_cell(res, &ColumnId::new(*id), now());
        assert_eq!(
            got.display(),
            *want,
            "{} {}: column `{id}`",
            res.kind.kind,
            res.meta.name
        );
    }
}

/// A namespaced `demo/x` object with the given top-level fields.
pub(super) fn namespaced(api_version: &str, kind: &str, fields: Value) -> Resource {
    build(resource(api_version, kind), fields)
}

/// A cluster-scoped object `x`.
pub(super) fn cluster(api_version: &str, kind: &str, fields: Value) -> Resource {
    build(resource(api_version, kind).cluster_scoped(), fields)
}

fn build(mut b: ResourceBuilder, fields: Value) -> Resource {
    b = b.name("x").created("2026-01-01T00:00:00Z");
    if let Value::Object(map) = fields {
        for (k, v) in map {
            b = b.field(k, v);
        }
    }
    b.build()
}
