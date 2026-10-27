//! [`RuleRow`]: one rule of a Role as the detail's table shows it.

use std::sync::Arc;

use oxikube_domain::rbac::PolicyRule;
use serde_json::{Map, Value, json};

use super::draft::VERBS;

/// What a wildcard is called in the table.
const ALL: &str = "all";

/// One row: the five columns, normalised for reading, and the rule as written for the hover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRow {
    /// API groups: `core` for the core group, `all` for `*`.
    pub api_groups: Arc<str>,
    /// Resources: `all` for `*`, `all/scale` for `*/scale`.
    pub resources: Arc<str>,
    /// Verbs: `all` when `*` is there or every verb of the dialog's boxes is.
    pub verbs: Arc<str>,
    /// Resource names, blank when the rule covers every object.
    pub resource_names: Arc<str>,
    /// Non-resource URLs.
    pub non_resource_urls: Arc<str>,
    /// The rule exactly as the object declares it, as indented JSON (the hover).
    pub raw: Arc<str>,
}

impl RuleRow {
    /// The row of `rule`.
    pub fn of(rule: &PolicyRule) -> Self {
        Self {
            api_groups: join(&rule.api_groups, group_name).into(),
            resources: join(&rule.resources, resource_name).into(),
            verbs: verbs_text(&rule.verbs).into(),
            resource_names: join(&rule.resource_names, str::to_owned).into(),
            non_resource_urls: join(&rule.non_resource_urls, str::to_owned).into(),
            raw: raw(rule).into(),
        }
    }

    /// The rows of `rules`, in the order written.
    pub fn list(rules: &[PolicyRule]) -> Vec<Self> {
        rules.iter().map(Self::of).collect()
    }
}

fn join(items: &[String], show: fn(&str) -> String) -> String {
    let shown: Vec<String> = items.iter().map(|item| show(item)).collect();
    shown.join(", ")
}

fn group_name(group: &str) -> String {
    match group {
        "" => "core".to_owned(),
        "*" => ALL.to_owned(),
        other => other.to_owned(),
    }
}

fn resource_name(resource: &str) -> String {
    match resource.strip_prefix("*/") {
        Some(sub) => format!("{ALL}/{sub}"),
        None if resource == "*" => ALL.to_owned(),
        None => resource.to_owned(),
    }
}

fn verbs_text(verbs: &[String]) -> String {
    let every = VERBS[..VERBS.len() - 1]
        .iter()
        .all(|verb| verbs.iter().any(|own| own == verb));
    if every || verbs.iter().any(|verb| verb == "*") {
        return ALL.to_owned();
    }
    verbs.join(", ")
}

fn raw(rule: &PolicyRule) -> String {
    let mut map = Map::new();
    for (key, values) in [
        ("apiGroups", &rule.api_groups),
        ("resources", &rule.resources),
        ("resourceNames", &rule.resource_names),
        ("nonResourceURLs", &rule.non_resource_urls),
        ("verbs", &rule.verbs),
    ] {
        if !values.is_empty() {
            map.insert(key.to_owned(), json!(values));
        }
    }
    serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default()
}
