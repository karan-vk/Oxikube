//! RBAC binding cells: RoleBinding and ClusterRoleBinding.

use jiff::Timestamp;
use oxikube_domain::Resource;

use super::{arr_at, str_at};
use crate::columns::Cell;

/// `ROLE`: `Kind/name` of `roleRef`.
pub(crate) fn binding_role<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let Some(role) = res.json.get("roleRef") else {
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
    let names: Vec<&str> = arr_at(&res.json, "subjects")
        .iter()
        .filter_map(|s| str_at(s, "name"))
        .collect();
    Cell::text(names.join(","))
}

/// `SUBJECT KINDS`: the distinct kinds of the subjects (`User`, `Group`, `ServiceAccount`).
pub(crate) fn binding_kinds<'a>(res: &'a Resource, _now: Timestamp) -> Cell<'a> {
    let mut kinds: Vec<&str> = Vec::new();
    for kind in arr_at(&res.json, "subjects")
        .iter()
        .filter_map(|s| str_at(s, "kind"))
    {
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    Cell::text(kinds.join(","))
}
