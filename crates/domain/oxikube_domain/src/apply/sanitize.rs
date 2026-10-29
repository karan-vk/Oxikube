//! [`sanitize_for_apply`]: strip what the server owns before an object is sent.

use serde_json::Value;

use super::strips::{COPY_GLOBAL, GLOBAL, PER_KIND};

/// How much to strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanitizeMode {
    /// A user-authored file: only the fields the server owns and rejects or ignores (status,
    /// uid, resourceVersion, managed fields, timestamps, the last-applied annotation). What the
    /// user wrote on purpose, a Service `clusterIP` or an owner reference, stays.
    Apply,
    /// An object read from a cluster, bound for another: [`Apply`](Self::Apply)'s strips, plus
    /// owner references and finalizers and the per-kind fields a controller allocated (Service
    /// `clusterIP` and `nodePort`, PVC `volumeName`, Pod `nodeName`, the Deployment revision
    /// annotation, a Job's generated selector). Labels and other annotations are kept.
    Copy,
}

/// What [`sanitize_for_apply`] removed: paths only, never values (the object may be a Secret).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SanitizeReport {
    /// Dotted paths of the removed fields, with `[i]` for array elements, in table order.
    /// An annotation or label key appears as one segment, for example
    /// `metadata.annotations.deployment.kubernetes.io/revision`.
    pub removed: Vec<String>,
}

impl SanitizeReport {
    /// Whether nothing was removed.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty()
    }
}

/// Removes server-owned fields from `object` in place, table-driven (see `strips`), and lists
/// what went. An annotations or labels map the strip leaves empty is dropped too (not reported).
pub fn sanitize_for_apply(object: &mut Value, mode: SanitizeMode) -> SanitizeReport {
    let mut report = SanitizeReport::default();
    for path in GLOBAL {
        strip(object, path, &mut report);
    }
    if mode == SanitizeMode::Copy {
        for path in COPY_GLOBAL {
            strip(object, path, &mut report);
        }
        let group_kind = group_kind(object);
        for row in PER_KIND {
            let applies = group_kind.0 == row.group && group_kind.1 == row.kind;
            if applies && !row.keep_when.is_some_and(|keep| keep(object)) {
                strip(object, row.path, &mut report);
            }
        }
    }
    report
}

/// `(group, kind)` of the object; the version is not needed.
fn group_kind(object: &Value) -> (String, String) {
    let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default();
    let api_version = text("apiVersion");
    let group = api_version.split_once('/').map_or("", |(group, _)| group);
    (group.to_owned(), text("kind").to_owned())
}

fn strip(object: &mut Value, path: &[&str], report: &mut SanitizeReport) {
    remove(object, path, String::new(), report);
}

fn remove(node: &mut Value, path: &[&str], at: String, report: &mut SanitizeReport) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    if *first == "[]" {
        if let Value::Array(items) = node {
            for (index, item) in items.iter_mut().enumerate() {
                remove(item, rest, format!("{at}[{index}]"), report);
            }
        }
        return;
    }
    let Value::Object(map) = node else {
        return;
    };
    let here = if at.is_empty() {
        (*first).to_owned()
    } else {
        format!("{at}.{first}")
    };
    if rest.is_empty() {
        if map.remove(*first).is_some() {
            report.removed.push(here);
        }
        return;
    }
    let Some(child) = map.get_mut(*first) else {
        return;
    };
    remove(child, rest, here, report);
    // Leave no empty `annotations: {}` / `labels: {}` behind a strip.
    if matches!(child, Value::Object(inner) if inner.is_empty())
        && matches!(*first, "annotations" | "labels")
    {
        map.remove(*first);
    }
}
