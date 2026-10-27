//! RBAC cells: Role, ClusterRole, RoleBinding and ClusterRoleBinding.

use jiff::Timestamp;
use oxikube_domain::Resource;
use oxikube_domain::json::JsonRef;

use super::{arr_at, str_at};
use crate::columns::Cell;

/// `ROLE`: `Kind/name` of `roleRef`.
pub(crate) fn binding_role<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(role) = res.json().get("roleRef") else {
        return Cell::empty();
    };
    match (str_at(role, "kind"), str_at(role, "name")) {
        (Some(kind), Some(name)) => Cell::text(format!("{kind}/{name}")),
        (None, Some(name)) => Cell::text(name),
        _ => Cell::empty(),
    }
}

/// `SUBJECTS`: the bound names, in order, comma-separated.
pub(crate) fn binding_subjects<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let names: Vec<&str> = arr_at(res.json(), "subjects")
        .iter()
        .filter_map(|s| str_at(s, "name"))
        .collect();
    Cell::text(names.join(","))
}

/// `SUBJECT KINDS`: the distinct kinds of the subjects (`User`, `Group`, `ServiceAccount`).
pub(crate) fn binding_kinds<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let mut kinds: Vec<&str> = Vec::new();
    for kind in arr_at(res.json(), "subjects")
        .iter()
        .filter_map(|s| str_at(s, "kind"))
    {
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    Cell::text(kinds.join(","))
}

/// `RULES`: how many rules the role holds.
pub(crate) fn role_rules<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let count = arr_at(res.json(), "rules").len();
    Cell::int(i64::try_from(count).unwrap_or(i64::MAX))
}

/// `AGGREGATED`: the selectors of an aggregated ClusterRole's `aggregationRule` as `key=value`
/// (an expression as `key in (a,b)`), `;`-separated; blank for any other role.
pub(crate) fn role_aggregated<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(rule) = res.json().get("aggregationRule") else {
        return Cell::empty();
    };
    let mut parts: Vec<String> = Vec::new();
    for selector in arr_at(rule, "clusterRoleSelectors").iter() {
        let mut terms: Vec<String> = Vec::new();
        if let Some(labels) = selector.get("matchLabels").and_then(JsonRef::as_object) {
            terms.extend(
                labels.iter().filter_map(|(key, value)| {
                    value.as_str().map(|value| format!("{key}={value}"))
                }),
            );
        }
        for expression in arr_at(selector, "matchExpressions").iter() {
            let (Some(key), Some(operator)) =
                (str_at(expression, "key"), str_at(expression, "operator"))
            else {
                continue;
            };
            let values: Vec<&str> = arr_at(expression, "values")
                .iter()
                .filter_map(JsonRef::as_str)
                .collect();
            terms.push(format!("{key} {operator} ({})", values.join(",")));
        }
        parts.push(terms.join(","));
    }
    Cell::text(parts.join("; "))
}
