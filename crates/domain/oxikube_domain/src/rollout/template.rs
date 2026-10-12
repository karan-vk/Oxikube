//! Pod template helpers: images, normalising before a comparison, and the changes between two
//! templates.

use serde_json::{Map, Value};

use super::HASH_LABEL;
use crate::Resource;
use crate::json::JsonRef;

/// The most changes a [`Revision`](super::Revision) lists: a preview, not a diff tool.
pub const MAX_CHANGES: usize = 12;

/// One difference between two pod templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateChange {
    /// Where, as a path (`containers[web].image`, `containers[web].env[LOG].value`).
    pub path: String,
    /// The value in the running template; `None` when the running template lacks it.
    pub from: Option<String>,
    /// The value in the revision's template; `None` when it lacks it.
    pub to: Option<String>,
}

/// The images of a pod template's containers, in order.
pub fn images_of(template: JsonRef<'_>) -> Vec<String> {
    template
        .pointer("/spec/containers")
        .and_then(JsonRef::as_array)
        .into_iter()
        .flatten()
        .filter_map(|container| container.get("image").and_then(JsonRef::as_str))
        .map(str::to_owned)
        .collect()
}

/// A template as it is compared: without the hash label the Deployment controller adds, the
/// `$patch` directive a ControllerRevision carries, and a null `creationTimestamp`.
pub(super) fn normalised(mut template: Value) -> Value {
    if let Some(object) = template.as_object_mut() {
        object.shift_remove("$patch");
    }
    if let Some(labels) = template
        .pointer_mut("/metadata/labels")
        .and_then(Value::as_object_mut)
    {
        labels.shift_remove(HASH_LABEL);
    }
    if let Some(metadata) = template
        .pointer_mut("/metadata")
        .and_then(Value::as_object_mut)
    {
        if metadata
            .get("creationTimestamp")
            .is_some_and(Value::is_null)
        {
            metadata.shift_remove("creationTimestamp");
        }
        if metadata
            .get("labels")
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty)
        {
            metadata.shift_remove("labels");
        }
    }
    template
}

/// The pod template `workload` runs now (`spec.template`), [`normalised`].
pub(super) fn running_template(workload: &Resource) -> Option<Value> {
    workload
        .get("/spec/template")
        .map(|template| normalised(template.to_value()))
}

/// The changes from the `running` template to the `target`, at most [`MAX_CHANGES`]. Both are
/// compared [`normalised`]. Arrays of objects with a `name` (containers, env, ports) are matched
/// by name, others by position.
pub(super) fn changes(running: &Value, target: &Value) -> Vec<TemplateChange> {
    let mut out = Vec::new();
    diff(&mut out, "", Some(running), Some(target));
    out
}

fn diff<'a>(
    out: &mut Vec<TemplateChange>,
    path: &str,
    from: Option<&'a Value>,
    to: Option<&'a Value>,
) {
    if out.len() >= MAX_CHANGES || from == to {
        return;
    }
    match (from, to) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            for key in a.keys().chain(b.keys().filter(|k| !a.contains_key(*k))) {
                diff(out, &join(path, key), a.get(key), b.get(key));
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) if named(a) && named(b) => {
            let name = |v: &Value| {
                v.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let find = |list: &'a [Value], n: &str| list.iter().find(|v| name(v) == n);
            let mut names: Vec<String> = a.iter().map(name).collect();
            for n in b.iter().map(name) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            for n in names {
                diff(out, &format!("{path}[{n}]"), find(a, &n), find(b, &n));
            }
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) => {
            for i in 0..a.len().max(b.len()) {
                diff(out, &format!("{path}[{i}]"), a.get(i), b.get(i));
            }
        }
        _ => {
            // An environment variable's literal value can be a password or a token: the preview
            // says it changed, never what it is.
            let hide = is_env_value(path);
            let render = |value: &Value| {
                if hide {
                    "(hidden)".to_owned()
                } else {
                    show(value)
                }
            };
            out.push(TemplateChange {
                path: path.to_owned(),
                from: from.map(render),
                to: to.map(render),
            });
        }
    }
}

/// Whether `path` is the literal `value` of an environment variable (`...env[LOG].value`).
fn is_env_value(path: &str) -> bool {
    path.ends_with(".value") && path.rsplit_once(".env[").is_some()
}

/// Whether every element is an object with a string `name`.
fn named(list: &[Value]) -> bool {
    !list.is_empty()
        && list
            .iter()
            .all(|v| v.get("name").is_some_and(Value::is_string))
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// A value as a short line: strings bare and redacted, scalars as JSON, a subtree as `{…}` / `[…]`.
fn show(value: &Value) -> String {
    match value {
        // Run through the secret scrubber: a `--token=...` argument is still a secret.
        Value::String(s) => crate::redact::redact(s).into_owned(),
        Value::Object(_) => "{…}".to_owned(),
        Value::Array(_) => "[…]".to_owned(),
        other => other.to_string(),
    }
}
