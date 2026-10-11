//! The CRD list's computed cells, read straight from the JSON, and Kubernetes' version priority.

use std::cmp::Ordering;

use super::super::{arr_of, bool_of, str_of, sub};
use crate::json::JsonRef;
use crate::resource::Resource;

/// The `Versions` cell of a CRD straight from its JSON, without building a [`CrdSummary`]
/// (the CRD list draws it per visible row). Same text as [`CrdSummary::versions_text`].
pub fn crd_versions_text(res: &Resource) -> String {
    let mut out = String::new();
    for v in arr_of(sub(res.json(), "spec"), "versions").iter() {
        if let Some(name) = str_of(v, "name")
            && bool_of(v, "served")
        {
            push_version(&mut out, name, bool_of(v, "storage"));
        }
    }
    out
}

/// The `Aliases` cell of a CRD straight from its JSON: plural, singular and short names,
/// comma-separated, without repeats. Same names as [`CrdSummary::aliases`].
pub fn crd_aliases_text(res: &Resource) -> String {
    let names = sub(sub(res.json(), "spec"), "names");
    let mut seen: Vec<&str> = Vec::new();
    let candidates = [str_of(names, "plural"), str_of(names, "singular")]
        .into_iter()
        .flatten()
        .chain(
            arr_of(names, "shortNames")
                .iter()
                .filter_map(JsonRef::as_str),
        );
    for name in candidates {
        if !name.is_empty() && !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen.join(",")
}

pub(super) fn push_version(out: &mut String, name: &str, storage: bool) {
    if !out.is_empty() {
        out.push(',');
    }
    out.push_str(name);
    if storage {
        out.push_str("(storage)");
    }
}

/// Kubernetes' version priority, as an `Ordering` for sorting newest first: GA (`v2`) before
/// beta (`v1beta2`) before alpha (`v1alpha1`), larger numbers first within a stability, and
/// names that are not versions last, alphabetically.
pub fn version_order(a: &str, b: &str) -> Ordering {
    match (parse_version(a), parse_version(b)) {
        // Newest first: the greater (stability, major, minor) sorts earlier.
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

/// Stability (alpha < beta < GA), major, minor of `v<major>[(alpha|beta)<minor>]`.
fn parse_version(name: &str) -> Option<(u8, u32, u32)> {
    let rest = name.strip_prefix('v')?;
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let major: u32 = rest[..digits].parse().ok()?;
    let tail = &rest[digits..];
    if tail.is_empty() {
        return Some((2, major, 0));
    }
    let (stability, minor) = if let Some(m) = tail.strip_prefix("alpha") {
        (0, m)
    } else if let Some(m) = tail.strip_prefix("beta") {
        (1, m)
    } else {
        return None;
    };
    Some((stability, major, minor.parse().ok()?))
}
