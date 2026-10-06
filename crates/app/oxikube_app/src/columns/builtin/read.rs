//! Reads a [`Src`] out of a [`Resource`]: the allocation-light path every cell goes through.
//!
//! Strings at a JSON pointer are borrowed from the object. Numbers allocate their text once.
//! Nothing here parses a pointer: `serde_json`'s `pointer` walks the tokens in place.

use jiff::Timestamp;
use oxikube_domain::{Age, ObjectMeta, Quantity, Resource};
use serde_json::Value;

use super::def::Src;
use crate::columns::Cell;

/// The cell of `src` for `res` at `now`. [`Src::Metric`] is not handled here (it needs the
/// registered source) and reads as pending.
pub(super) fn read<'a>(src: Src, res: &'a Resource, now: Timestamp) -> Cell<'a> {
    match src {
        Src::Name => name(&res.meta),
        Src::Namespace => namespace(&res.meta),
        Src::Age => age(&res.meta, now),
        Src::Labels => labels(&res.meta),
        Src::Text(ptr) => res.json.pointer(ptr).map_or_else(Cell::empty, scalar),
        Src::Int(ptr) => res
            .json
            .pointer(ptr)
            .and_then(Value::as_i64)
            .map_or_else(Cell::empty, Cell::int),
        Src::Qty(ptr) => res
            .json
            .pointer(ptr)
            .and_then(Value::as_str)
            .map_or_else(Cell::empty, quantity),
        Src::Len(ptr) => Cell::int(len_at(&res.json, ptr)),
        Src::Func(f) => f(res, now),
        Src::Metric(_) => Cell::Pending,
    }
}

/// `metadata.name`.
pub(crate) fn name(meta: &ObjectMeta) -> Cell<'_> {
    Cell::text(&*meta.name)
}

/// `metadata.namespace`; blank for a cluster-scoped object.
pub(crate) fn namespace(meta: &ObjectMeta) -> Cell<'_> {
    meta.namespace
        .as_deref()
        .map_or_else(Cell::empty, Cell::text)
}

/// Time since creation; blank when the object has no creation timestamp.
pub(crate) fn age<'a>(meta: &ObjectMeta, now: Timestamp) -> Cell<'a> {
    meta.creation
        .map_or_else(Cell::empty, |created| Cell::age(Age::between(created, now)))
}

/// `k=v,k=v` in key order; blank when there are no labels.
pub(crate) fn labels<'a>(meta: &ObjectMeta) -> Cell<'a> {
    Cell::text(key_values(meta.labels.iter().map(|(k, v)| (&**k, &**v))))
}

/// `k=v,k=v` in the order given.
pub(crate) fn key_values<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut out = String::new();
    for (k, v) in pairs {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
}

/// A scalar JSON value as a text cell; arrays and objects are blank.
pub(crate) fn scalar(v: &Value) -> Cell<'_> {
    match v {
        Value::String(s) => Cell::text(s.as_str()),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Cell::int(i),
            (None, Some(f)) => Cell::float(n.to_string(), f),
            (None, None) => Cell::text(n.to_string()),
        },
        Value::Bool(b) => Cell::text(if *b { "true" } else { "false" }),
        Value::Null | Value::Array(_) | Value::Object(_) => Cell::empty(),
    }
}

/// A quantity string shown as written and sorted by value; unparseable text sorts as text.
pub(crate) fn quantity(text: &str) -> Cell<'_> {
    match Quantity::parse(text) {
        Ok(q) => Cell::quantity(text, q),
        Err(_) => Cell::text(text),
    }
}

/// Entries of the array or object at `ptr`.
pub(crate) fn len_at(json: &Value, ptr: &str) -> i64 {
    let n = match json.pointer(ptr) {
        Some(Value::Array(a)) => a.len(),
        Some(Value::Object(o)) => o.len(),
        _ => 0,
    };
    i64::try_from(n).unwrap_or(i64::MAX)
}
