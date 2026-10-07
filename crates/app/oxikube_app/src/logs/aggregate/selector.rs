//! Turning an object's pod selector into the label selector string the API server takes.
//!
//! `spec.selector` is a `LabelSelector` (`matchLabels` + `matchExpressions`) on Deployment,
//! StatefulSet, DaemonSet, ReplicaSet and Job, and a plain `key: value` map on Service and
//! ReplicationController. Pure functions over the JSON of the object.

use oxikube_domain::{OxiError, OxiResult, Resource};
use serde_json::Value;

/// The label selector (`a=b,c in (d,e),!f`) of the pods `object` selects.
///
/// # Errors
///
/// A validation error when the object has no selector (a Service without one routes by hand) or
/// an empty one (which would select every pod of the namespace), or a selector this cannot
/// express.
pub fn selector_of(object: &Resource) -> OxiResult<String> {
    let what = format!("{} {}", object.kind.kind, object.name());
    let Some(selector) = object.json.pointer("/spec/selector") else {
        return Err(OxiError::validation(format!("{what} has no selector")));
    };
    let mut terms = Vec::new();
    if selector.get("matchLabels").is_some() || selector.get("matchExpressions").is_some() {
        if let Some(labels) = selector.get("matchLabels") {
            terms.extend(equality_terms(labels));
        }
        if let Some(Value::Array(expressions)) = selector.get("matchExpressions") {
            for expression in expressions {
                terms.push(expression_term(expression, &what)?);
            }
        }
    } else {
        terms.extend(equality_terms(selector));
    }
    if terms.is_empty() {
        return Err(OxiError::validation(format!(
            "{what} has an empty selector: it would select every pod"
        )));
    }
    Ok(terms.join(","))
}

/// `a` AND `b` as one label selector; `b` is optional.
pub fn and_selectors(a: &str, b: Option<&str>) -> String {
    match b.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => format!("{a},{b}"),
        None => a.to_owned(),
    }
}

/// `key=value` for each string entry of a map, sorted by key.
fn equality_terms(map: &Value) -> Vec<String> {
    let Some(map) = map.as_object() else {
        return Vec::new();
    };
    let mut terms: Vec<String> = map
        .iter()
        .filter_map(|(key, value)| Some(format!("{key}={}", value.as_str()?)))
        .collect();
    terms.sort();
    terms
}

fn expression_term(expression: &Value, what: &str) -> OxiResult<String> {
    let key = expression.get("key").and_then(Value::as_str);
    let operator = expression.get("operator").and_then(Value::as_str);
    let values: Vec<&str> = expression
        .get("values")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match (key, operator) {
        (Some(key), Some("In")) if !values.is_empty() => {
            Ok(format!("{key} in ({})", values.join(",")))
        }
        (Some(key), Some("NotIn")) if !values.is_empty() => {
            Ok(format!("{key} notin ({})", values.join(",")))
        }
        (Some(key), Some("Exists")) => Ok(key.to_owned()),
        (Some(key), Some("DoesNotExist")) => Ok(format!("!{key}")),
        _ => Err(OxiError::validation(format!(
            "{what} has a selector expression this cannot read: {expression}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ErrorKind;
    use serde_json::json;

    use super::*;

    fn object(kind: &str, spec: Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": if kind == "Service" { "v1" } else { "apps/v1" },
            "kind": kind,
            "metadata": {"name": "api", "namespace": "shop"},
            "spec": spec,
        }))
        .unwrap()
    }

    #[test]
    fn match_labels_are_sorted_equality_terms() {
        let deployment = object(
            "Deployment",
            json!({"selector": {"matchLabels": {"tier": "api", "app": "web"}}}),
        );
        assert_eq!(selector_of(&deployment).unwrap(), "app=web,tier=api");
    }

    #[test]
    fn match_expressions_follow_the_labels_in_selector_syntax() {
        let deployment = object(
            "Deployment",
            json!({"selector": {
                "matchLabels": {"app": "web"},
                "matchExpressions": [
                    {"key": "env", "operator": "In", "values": ["prod", "staging"]},
                    {"key": "tier", "operator": "NotIn", "values": ["db"]},
                    {"key": "canary", "operator": "Exists"},
                    {"key": "legacy", "operator": "DoesNotExist"},
                ]
            }}),
        );
        assert_eq!(
            selector_of(&deployment).unwrap(),
            "app=web,env in (prod,staging),tier notin (db),canary,!legacy"
        );
    }

    #[test]
    fn a_service_selector_is_a_plain_map() {
        let service = object(
            "Service",
            json!({"selector": {"app": "web", "tier": "api"}}),
        );
        assert_eq!(selector_of(&service).unwrap(), "app=web,tier=api");
    }

    #[test]
    fn a_missing_empty_or_unreadable_selector_is_a_validation_error() {
        for spec in [
            json!({}),
            json!({"selector": {}}),
            json!({"selector": {"matchLabels": {}}}),
            json!({"selector": {"matchExpressions": [{"key": "a", "operator": "In"}]}}),
            json!({"selector": {"matchExpressions": [{"key": "a", "operator": "Bogus"}]}}),
        ] {
            let error = selector_of(&object("Deployment", spec.clone())).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Validation, "{spec}");
        }
    }

    #[test]
    fn a_narrowing_selector_is_anded() {
        assert_eq!(and_selectors("app=web", None), "app=web");
        assert_eq!(and_selectors("app=web", Some(" ")), "app=web");
        assert_eq!(
            and_selectors("app=web", Some("tier=api")),
            "app=web,tier=api"
        );
    }
}
