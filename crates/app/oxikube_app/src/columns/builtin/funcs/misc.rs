//! Cells of the remaining kinds: HorizontalPodAutoscaler, Event, CustomResourceDefinition and
//! the admission and scheduling kinds.

use jiff::Timestamp;
use oxikube_domain::{Age, Resource};
use serde_json::Value;

use super::{arr_at, spec, status, str_at};
use crate::columns::{Cell, Tone};

/// HPA `REFERENCE`: `Kind/name` of `spec.scaleTargetRef`.
pub(crate) fn hpa_reference<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(target) = spec(res).get("scaleTargetRef") else {
        return Cell::empty();
    };
    match (str_at(target, "kind"), str_at(target, "name")) {
        (Some(kind), Some(name)) => Cell::text(format!("{kind}/{name}")),
        _ => Cell::empty(),
    }
}

/// HPA `TARGETS`: `current/target` per metric, comma-separated (`40%/80%`, `<unknown>/80%`).
/// Reads the `autoscaling/v2` metric list, or the v1 CPU percentage fields.
pub(crate) fn hpa_targets<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let (spec, status) = (spec(res), status(res));
    let metrics = arr_at(spec, "metrics");
    if metrics.is_empty() {
        let target = spec
            .get("targetCPUUtilizationPercentage")
            .and_then(Value::as_i64);
        let current = status
            .get("currentCPUUtilizationPercentage")
            .and_then(Value::as_i64);
        return match target {
            Some(t) => Cell::text(format!(
                "{}/{t}%",
                current.map_or_else(|| "<unknown>".to_owned(), |c| format!("{c}%"))
            )),
            None => Cell::empty(),
        };
    }
    let current_metrics = arr_at(status, "currentMetrics");
    let parts: Vec<String> = metrics
        .iter()
        .map(|m| {
            let target = metric_value(m, "target").unwrap_or_else(|| "<unknown>".to_owned());
            let current = current_metrics
                .iter()
                .find(|c| same_metric(m, c))
                .and_then(|c| metric_value(c, "current"))
                .unwrap_or_else(|| "<unknown>".to_owned());
            format!("{current}/{target}")
        })
        .collect();
    Cell::text(parts.join(", "))
}

/// The `type` of an HPA metric and the object holding its fields (`resource`, `pods`, ...).
fn metric_body(m: &Value) -> Option<(&str, &Value)> {
    let kind = str_at(m, "type")?;
    let key = match kind {
        "Resource" => "resource",
        "ContainerResource" => "containerResource",
        "Pods" => "pods",
        "Object" => "object",
        "External" => "external",
        _ => return None,
    };
    Some((kind, m.get(key)?))
}

/// The metric's name: the resource name, or `metric.name` for the custom kinds.
fn metric_name(body: &Value) -> Option<&str> {
    str_at(body, "name").or_else(|| str_at(body.get("metric")?, "name"))
}

fn same_metric(spec_metric: &Value, current: &Value) -> bool {
    match (metric_body(spec_metric), metric_body(current)) {
        (Some((k1, b1)), Some((k2, b2))) => k1 == k2 && metric_name(b1) == metric_name(b2),
        _ => false,
    }
}

/// `averageUtilization` (as a percentage), `averageValue` or `value` of `body[which]`.
fn metric_value(m: &Value, which: &str) -> Option<String> {
    let (_, body) = metric_body(m)?;
    let v = body.get(which)?;
    if let Some(u) = v.get("averageUtilization").and_then(Value::as_i64) {
        return Some(format!("{u}%"));
    }
    str_at(v, "averageValue")
        .or_else(|| str_at(v, "value"))
        .map(str::to_owned)
}

/// Event `LAST SEEN`: time since the event last happened. `series.lastObservedTime` wins (a
/// series event's `eventTime` is its first observation), then `lastTimestamp`, then `eventTime`.
pub(crate) fn event_last_seen<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    let json = &res.json;
    let at = json
        .pointer("/series/lastObservedTime")
        .and_then(Value::as_str)
        .or_else(|| str_at(json, "lastTimestamp"))
        .or_else(|| str_at(json, "eventTime"))
        .and_then(|s| s.parse::<Timestamp>().ok())
        .or(res.meta.creation);
    at.map_or_else(Cell::empty, |t| Cell::age(Age::between(t, now)))
}

/// Event `TYPE`: `Normal` or `Warning`; warnings are amber.
pub(crate) fn event_type<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    match str_at(&res.json, "type") {
        Some("Warning") => Cell::text("Warning").with_tone(Tone::Warn),
        Some(other) => Cell::text(other),
        None => Cell::empty(),
    }
}

/// Event `OBJECT`: `kind/name` of the involved object, kind in lower case as kubectl prints it.
pub(crate) fn event_object<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(obj) = res.json.get("involvedObject") else {
        return Cell::empty();
    };
    match (str_at(obj, "kind"), str_at(obj, "name")) {
        (Some(kind), Some(name)) => Cell::text(format!("{}/{name}", kind.to_ascii_lowercase())),
        _ => Cell::empty(),
    }
}

/// CRD `VERSION`: the storage version, else the first served one.
pub(crate) fn crd_version<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let versions = arr_at(spec(res), "versions");
    versions
        .iter()
        .find(|v| v.get("storage").and_then(Value::as_bool) == Some(true))
        .or_else(|| versions.first())
        .and_then(|v| str_at(v, "name"))
        .map_or_else(Cell::empty, Cell::text)
}

/// CRD `SHORT NAMES`: `spec.names.shortNames`, comma-separated.
pub(crate) fn crd_short_names<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let names = res.json.pointer("/spec/names").unwrap_or(&Value::Null);
    let list: Vec<&str> = arr_at(names, "shortNames")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    Cell::text(list.join(","))
}

/// `ACTIONS` of a ValidatingAdmissionPolicyBinding: `spec.validationActions`.
pub(crate) fn binding_actions<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let list: Vec<&str> = arr_at(spec(res), "validationActions")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    Cell::text(list.join(","))
}
