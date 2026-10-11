//! Cells of the remaining kinds: HorizontalPodAutoscaler, VerticalPodAutoscaler, Event,
//! CustomResourceDefinition and the admission and scheduling kinds.

use jiff::Timestamp;
use oxikube_domain::json::JsonRef;
use oxikube_domain::view::{HpaState, HpaSummary, PdbSummary, VpaSummary};
use oxikube_domain::view::{crd_aliases_text, crd_versions_text};
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
    Cell::text(hpa.state.text().to_owned()).with_tone(hpa_state_tone(&hpa.state))
}

/// The tone of an autoscaler's state: the status column and the detail panel share it.
pub fn hpa_state_tone(state: &HpaState) -> Tone {
    match state {
        HpaState::Active => Tone::Ok,
        HpaState::Limited(_) => Tone::Warn,
        HpaState::Inactive(_) => Tone::Error,
        HpaState::Disabled | HpaState::Unknown => Tone::Neutral,
    }
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

/// CRD `VERSIONS`: the served versions, the storage one marked (`v1(storage),v1beta1`), as the
/// API server's own CRD table prints them.
pub(crate) fn crd_versions<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(crd_versions_text(res))
}

/// CRD `ALIASES` (k9s): plural, singular and short names, what `:` accepts for the kind.
pub(crate) fn crd_aliases<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    Cell::text(crd_aliases_text(res))
}

/// CRD `ESTABLISHED`: the `Established` condition, toned (`True` ok, `False` warn).
pub(crate) fn crd_established<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    // The last entry wins, as everywhere conditions are read.
    let established = arr_at(status(res), "conditions")
        .iter()
        .filter(|c| str_at(*c, "type") == Some("Established"))
        .last()
        .and_then(|c| str_at(c, "status"));
    match established {
        Some("True") => Cell::text("True").with_tone(Tone::Ok),
        Some(other) => Cell::text(other).with_tone(Tone::Warn),
        None => Cell::empty(),
    }
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
