//! A workload's selector as a list selector, and who owns whom.

use crate::json::JsonRef;
use crate::{OxiError, OxiResult, Resource};

/// The workload's `spec.selector` as a label selector string
/// (`app=web,tier in (a,b),!canary`), the filter for listing its ReplicaSets or
/// ControllerRevisions.
///
/// # Errors
///
/// `Validation` when the workload has no selector (the API server never allows that) or an
/// expression with an operator this does not know.
pub fn label_selector(workload: &Resource) -> OxiResult<String> {
    let selector = workload.get("/spec/selector");
    let mut terms: Vec<String> = Vec::new();
    if let Some(labels) = selector
        .and_then(|s| s.get("matchLabels"))
        .and_then(JsonRef::as_object)
    {
        for (key, value) in labels.iter() {
            terms.push(format!("{key}={}", value.as_str().unwrap_or_default()));
        }
    }
    let expressions = selector
        .and_then(|s| s.get("matchExpressions"))
        .and_then(JsonRef::as_array);
    for expression in expressions.into_iter().flatten() {
        let key = expression
            .get("key")
            .and_then(JsonRef::as_str)
            .unwrap_or("");
        let values = expression
            .get("values")
            .and_then(JsonRef::as_array)
            .map(|vs| {
                vs.iter()
                    .filter_map(JsonRef::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        terms.push(match expression.get("operator").and_then(JsonRef::as_str) {
            Some("In") => format!("{key} in ({values})"),
            Some("NotIn") => format!("{key} notin ({values})"),
            Some("Exists") => key.to_owned(),
            Some("DoesNotExist") => format!("!{key}"),
            _ => {
                return Err(OxiError::validation(format!(
                    "{} {} has a selector expression with an unknown operator",
                    workload.kind.kind.to_lowercase(),
                    workload.name()
                )));
            }
        });
    }
    if terms.is_empty() {
        return Err(OxiError::validation(format!(
            "{} {} has no selector",
            workload.kind.kind.to_lowercase(),
            workload.name()
        )));
    }
    Ok(terms.join(","))
}

/// Whether `object` names `owner` as its controller: the kind matches, and so does the uid when
/// both have one (else the name).
pub(super) fn is_controlled_by(object: &Resource, owner: &Resource) -> bool {
    object.meta.controller_ref().is_some_and(|reference| {
        *reference.kind == *owner.kind.kind
            && match (&owner.meta.uid, &reference.uid) {
                (Some(want), have) if !have.is_empty() => want == have,
                _ => *reference.name == *owner.name(),
            }
    })
}
