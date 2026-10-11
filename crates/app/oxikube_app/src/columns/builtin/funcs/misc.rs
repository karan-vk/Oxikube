//! Cells of the remaining kinds: HorizontalPodAutoscaler, VerticalPodAutoscaler, Event,
//! CustomResourceDefinition and the admission and scheduling kinds.

use jiff::Timestamp;
use oxikube_domain::json::JsonRef;
use oxikube_domain::view::{HpaState, HpaSummary, PdbSummary, VpaSummary};
use oxikube_domain::{Age, Resource};

use super::{arr_at, spec, str_at};
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

/// HPA `METRICS` (`TARGETS` in kubectl): `current/target` per metric, comma-separated
/// (`40%/80%`, `<unknown>/80%`). The text is the domain view-model's, so the detail panel and the
/// column cannot disagree.
pub(crate) fn hpa_targets<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    HpaSummary::from_resource(res)
        .ok()
        .and_then(|hpa| hpa.metrics_text())
        .map_or_else(Cell::empty, Cell::text)
}

/// HPA `STATUS`: `Active`, the reason it is held at a bound, or the reason it cannot scale.
pub(crate) fn hpa_status<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Ok(hpa) = HpaSummary::from_resource(res) else {
        return Cell::empty();
    };
    let tone = match hpa.state {
        HpaState::Active => Tone::Ok,
        HpaState::Limited(_) => Tone::Warn,
        HpaState::Inactive(_) => Tone::Error,
        HpaState::Disabled | HpaState::Unknown => Tone::Neutral,
    };
    Cell::text(hpa.state.text().to_owned()).with_tone(tone)
}

/// VPA `REFERENCE`: `Kind/name` of `spec.targetRef`.
pub(crate) fn vpa_reference<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(target) = spec(res).get("targetRef") else {
        return Cell::empty();
    };
    match (str_at(target, "kind"), str_at(target, "name")) {
        (Some(kind), Some(name)) => Cell::text(format!("{kind}/{name}")),
        _ => Cell::empty(),
    }
}

/// VPA `MODE`: `spec.updatePolicy.updateMode`, `Auto` when unset (the API default).
pub(crate) fn vpa_mode<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    VpaSummary::from_resource(res)
        .map_or_else(|_| Cell::empty(), |vpa| Cell::text(vpa.mode.to_string()))
}

/// VPA `CONDITIONS`: the types of the conditions that hold, comma-separated.
pub(crate) fn vpa_conditions<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    VpaSummary::from_resource(res)
        .map_or_else(|_| Cell::empty(), |vpa| Cell::text(vpa.conditions_text()))
}

/// PDB `ALLOWED DISRUPTIONS`: the controller's `disruptionsAllowed`, else the healthy pods above
/// the desired count (the same rule the detail panel shows).
pub(crate) fn pdb_allowed<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    PdbSummary::from_resource(res).map_or_else(
        |_| Cell::empty(),
        |pdb| Cell::int(pdb.allowed_disruptions()),
    )
}

/// Event `LAST SEEN`: time since the event last happened. `series.lastObservedTime` wins (a
/// series event's `eventTime` is its first observation), then `lastTimestamp`, then `eventTime`.
pub(crate) fn event_last_seen<'a>(res: &'a Resource, now: Timestamp) -> Cell<'a> {
    let json = res.json();
    let at = json
        .pointer("/series/lastObservedTime")
        .and_then(JsonRef::as_str)
        .or_else(|| str_at(json, "lastTimestamp"))
        .or_else(|| str_at(json, "eventTime"))
        .and_then(|s| s.parse::<Timestamp>().ok())
        .or(res.meta.creation);
    at.map_or_else(Cell::empty, |t| Cell::age(Age::between(t, now)))
}

/// Event `TYPE`: `Normal` or `Warning`; warnings are amber.
pub(crate) fn event_type<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    match str_at(res.json(), "type") {
        Some("Warning") => Cell::text("Warning").with_tone(Tone::Warn),
        Some(other) => Cell::text(other),
        None => Cell::empty(),
    }
}

/// Event `OBJECT`: `kind/name` of the involved object, kind in lower case as kubectl prints it.
pub(crate) fn event_object<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(obj) = res.json().get("involvedObject") else {
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
        .find(|v| v.get("storage").and_then(JsonRef::as_bool) == Some(true))
        .or_else(|| versions.first())
        .and_then(|v| str_at(v, "name"))
        .map_or_else(Cell::empty, Cell::text)
}

/// CRD `SHORT NAMES`: `spec.names.shortNames`, comma-separated.
pub(crate) fn crd_short_names<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let names = res.json().pointer("/spec/names").unwrap_or(JsonRef::NULL);
    let list: Vec<&str> = arr_at(names, "shortNames")
        .iter()
        .filter_map(JsonRef::as_str)
        .collect();
    Cell::text(list.join(","))
}

/// `ACTIONS` of a ValidatingAdmissionPolicyBinding: `spec.validationActions`.
pub(crate) fn binding_actions<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let list: Vec<&str> = arr_at(spec(res), "validationActions")
        .iter()
        .filter_map(JsonRef::as_str)
        .collect();
    Cell::text(list.join(","))
}
