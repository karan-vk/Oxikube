//! Small readers and formatters the Config view-models share.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::super::{arc_of, arr_of, str_of};
use crate::ids::Gvk;
use crate::json::JsonRef;

/// A reference to another object by `apiVersion`, `kind` and `name`: a scale target, a param
/// kind. The namespace is the referring object's own, so it is not carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRef {
    /// `apiVersion` as written (`apps/v1`; empty when the reference carries none).
    pub api_version: Arc<str>,
    /// `kind`, for example `Deployment`.
    pub kind: Arc<str>,
    /// `name`; empty for a kind-only reference (a policy's param kind).
    pub name: Arc<str>,
}

impl TargetRef {
    /// Reads `{apiVersion, kind, name}`; `None` without a kind.
    pub(super) fn read(v: JsonRef<'_>) -> Option<Self> {
        Some(Self {
            api_version: arc_of(v, "apiVersion").unwrap_or_else(|| Arc::from("")),
            kind: arc_of(v, "kind")?,
            name: arc_of(v, "name").unwrap_or_else(|| Arc::from("")),
        })
    }

    /// The reference's group-version-kind.
    pub fn gvk(&self) -> Gvk {
        Gvk::from_api_version(&self.api_version, &self.kind)
    }

    /// `Kind/name` as kubectl prints a reference (`Deployment/web`); just the kind without a name.
    pub fn display(&self) -> String {
        if self.name.is_empty() {
            self.kind.to_string()
        } else {
            format!("{}/{}", self.kind, self.name)
        }
    }
}

/// A resource list (`hard`, `default`, `podFixed`) as sorted `(resource, quantity text)` pairs.
pub(super) fn quantities(v: JsonRef<'_>, key: &str) -> BTreeMap<Arc<str>, Arc<str>> {
    let mut out = BTreeMap::new();
    if let Some(map) = v.get(key).and_then(JsonRef::as_object) {
        for (name, value) in map.iter() {
            if let Some(text) = scalar_text(value) {
                out.insert(Arc::from(name), Arc::from(text));
            }
        }
    }
    out
}

/// A string or number as text (quantities are strings, but counts may arrive as numbers).
pub(super) fn scalar_text(v: JsonRef<'_>) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    if let Some(i) = v.as_i64() {
        return Some(i.to_string());
    }
    v.as_f64().map(|f| f.to_string())
}

/// `v[key]` as a list of strings.
pub(super) fn strings(v: JsonRef<'_>, key: &str) -> Vec<Arc<str>> {
    arr_of(v, key)
        .iter()
        .filter_map(JsonRef::as_str)
        .map(Arc::from)
        .collect()
}

/// A label selector as text: `app=web,tier in (a,b),!debug`. `None` when `v` is not an object;
/// an empty selector (which selects everything) reads `<all>`.
pub fn selector_text(v: JsonRef<'_>) -> Option<String> {
    if !v.is_object() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(labels) = v.get("matchLabels").and_then(JsonRef::as_object) {
        let mut pairs: Vec<(&str, &str)> = labels
            .iter()
            .map(|(k, val)| (k, val.as_str().unwrap_or_default()))
            .collect();
        pairs.sort_unstable();
        parts.extend(pairs.into_iter().map(|(k, val)| format!("{k}={val}")));
    }
    for req in arr_of(v, "matchExpressions").iter() {
        let Some(key) = str_of(req, "key") else {
            continue;
        };
        let values = || {
            arr_of(req, "values")
                .iter()
                .filter_map(JsonRef::as_str)
                .collect::<Vec<_>>()
                .join(",")
        };
        parts.push(match str_of(req, "operator") {
            Some("In") => format!("{key} in ({})", values()),
            Some("NotIn") => format!("{key} notin ({})", values()),
            Some("Exists") => key.to_owned(),
            Some("DoesNotExist") => format!("!{key}"),
            Some(other) => format!("{key} {other} ({})", values()),
            None => continue,
        });
    }
    Some(if parts.is_empty() {
        "<all>".to_owned()
    } else {
        parts.join(",")
    })
}
