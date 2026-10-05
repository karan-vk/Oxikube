//! Finding a Deployment's ReplicaSets and reading their revisions.

use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourcePort};
use serde_json::Value;

use crate::algorithms::api::{list_all, replicaset_gvk};

/// The annotation holding a revision number, on the Deployment (current) and on each of its
/// ReplicaSets.
pub const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";

/// The label the Deployment controller adds to a ReplicaSet and its pods to tell its
/// ReplicaSets apart; never part of the Deployment's own template.
pub(super) const HASH_LABEL: &str = "pod-template-hash";

/// A revision number: the annotation parsed, `None` when absent or not a number.
pub(super) fn revision_of(object: &Resource) -> Option<i64> {
    object
        .meta
        .annotations
        .get(REVISION_ANNOTATION)?
        .parse()
        .ok()
}

/// The ReplicaSets the Deployment controls, each with its revision, oldest revision first.
/// ReplicaSets without a readable revision are not part of the history.
pub(super) async fn revisions(
    port: &dyn ResourcePort,
    deployment: &Resource,
) -> OxiResult<Vec<(i64, Resource)>> {
    let selector = label_selector(deployment)?;
    let options = ListOptions::default().labels(selector).limit(500);
    let replicasets = list_all(port, &replicaset_gvk(), deployment.namespace(), options).await?;
    let mut found: Vec<(i64, Resource)> = replicasets
        .into_iter()
        .filter(|rs| is_controlled_by(rs, deployment))
        .filter_map(|rs| Some((revision_of(&rs)?, rs)))
        .collect();
    found.sort_by_key(|(revision, _)| *revision);
    Ok(found)
}

/// Whether `rs` names `deployment` as its controller (by uid when both have one).
fn is_controlled_by(rs: &Resource, deployment: &Resource) -> bool {
    rs.meta.controller_ref().is_some_and(|owner| {
        &*owner.kind == "Deployment"
            && match (&deployment.meta.uid, &owner.uid) {
                (Some(want), have) if !have.is_empty() => want == have,
                _ => *owner.name == *deployment.name(),
            }
    })
}

/// The Deployment's `spec.selector` as a label selector string
/// (`app=web,tier in (a,b),!canary`).
///
/// # Errors
///
/// `Validation` if the Deployment has no selector (the API server never allows that).
fn label_selector(deployment: &Resource) -> OxiResult<String> {
    let selector = deployment.get("/spec/selector");
    let mut terms: Vec<String> = Vec::new();
    if let Some(labels) = selector
        .and_then(|s| s.get("matchLabels"))
        .and_then(Value::as_object)
    {
        for (key, value) in labels {
            terms.push(format!("{key}={}", value.as_str().unwrap_or_default()));
        }
    }
    let expressions = selector
        .and_then(|s| s.get("matchExpressions"))
        .and_then(Value::as_array);
    for expression in expressions.into_iter().flatten() {
        let key = expression.get("key").and_then(Value::as_str).unwrap_or("");
        let values = expression
            .get("values")
            .and_then(Value::as_array)
            .map(|vs| {
                vs.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        terms.push(match expression.get("operator").and_then(Value::as_str) {
            Some("In") => format!("{key} in ({values})"),
            Some("NotIn") => format!("{key} notin ({values})"),
            Some("Exists") => key.to_owned(),
            Some("DoesNotExist") => format!("!{key}"),
            _ => {
                return Err(OxiError::validation(format!(
                    "deployment {} has a selector expression with an unknown operator",
                    deployment.name()
                )));
            }
        });
    }
    if terms.is_empty() {
        return Err(OxiError::validation(format!(
            "deployment {} has no selector",
            deployment.name()
        )));
    }
    Ok(terms.join(","))
}

/// The images of a pod template's containers.
pub(super) fn images(template: &Value) -> Vec<String> {
    template
        .pointer("/spec/containers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|container| container.get("image").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod unit {
    use serde_json::json;

    use super::*;

    fn deployment(selector: Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "web", "namespace": "default"},
            "spec": {"selector": selector},
        }))
        .expect("deployment")
    }

    #[test]
    fn selector_covers_labels_and_every_expression_operator() {
        let d = deployment(json!({
            "matchLabels": {"app": "web"},
            "matchExpressions": [
                {"key": "tier", "operator": "In", "values": ["a", "b"]},
                {"key": "env", "operator": "NotIn", "values": ["dev"]},
                {"key": "stable", "operator": "Exists"},
                {"key": "canary", "operator": "DoesNotExist"},
            ],
        }));
        assert_eq!(
            label_selector(&d).expect("selector"),
            "app=web,tier in (a,b),env notin (dev),stable,!canary"
        );
    }

    #[test]
    fn an_empty_or_unknown_selector_is_a_validation_error() {
        assert!(label_selector(&deployment(json!({}))).is_err());
        let odd = deployment(json!({"matchExpressions": [{"key": "k", "operator": "Gt"}]}));
        assert!(label_selector(&odd).is_err());
    }
}
