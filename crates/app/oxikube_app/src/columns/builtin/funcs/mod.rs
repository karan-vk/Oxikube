//! Computed core cells, one file per area. Each function is a [`CellFn`](super::def::CellFn):
//! `fn(&Resource, now) -> Cell` that reads the JSON by key, borrows strings and allocates only
//! for text it has to build.

mod batch;
mod discovery;
mod misc;
mod node;
mod pod;
mod rbac;
mod storage;
mod workload;

use std::fmt::Write as _;

use oxikube_domain::Resource;
use oxikube_domain::json::{Array, JsonRef};

pub(super) use batch::*;
pub(super) use discovery::*;
pub(super) use misc::*;
pub(super) use node::*;
pub(super) use pod::*;
pub(super) use rbac::*;
pub(super) use storage::*;
pub(super) use workload::*;

use super::read::key_values;
use crate::columns::Tone;

/// The tone of a status word (`Running`, `CrashLoopBackOff`, `Bound`, `Init:1/2`...), by the
/// words kubectl, Lens and k9s colour: failures red, transitions amber, healthy green, and
/// anything unrecognised neutral.
pub(crate) fn status_tone(status: &str) -> Tone {
    const ERROR: &[&str] = &[
        "Error",
        "Err",
        "BackOff",
        "Fail",
        "OOMKilled",
        "Evicted",
        "Invalid",
        "NotReady",
        "Lost",
        "Unschedulable",
        "Rejected",
        "CreateContainer",
        "RunContainerError",
        "ContainerStatusUnknown",
    ];
    const OK: &[&str] = &[
        "Running",
        "Completed",
        "Complete",
        "Succeeded",
        "Bound",
        "Active",
        "Ready",
        "Available",
        "Established",
        "Healthy",
        "Normal",
        "True",
    ];
    if ERROR.iter().any(|w| status.contains(w)) {
        Tone::Error
    } else if OK.contains(&status) {
        Tone::Ok
    } else if status.starts_with("Init:")
        || status.contains("Terminating")
        || status.contains("SchedulingDisabled")
        || matches!(
            status,
            "Pending"
                | "ContainerCreating"
                | "PodInitializing"
                | "Unknown"
                | "Released"
                | "Suspended"
                | "SchedulingGated"
                | "Warning"
                | "Paused"
                | "Progressing"
        )
    {
        Tone::Warn
    } else {
        Tone::Neutral
    }
}

/// The tone of a `have/want` readiness pair: green when complete, amber when short.
pub(crate) fn ready_tone(have: u32, want: u32) -> Tone {
    if have >= want { Tone::Ok } else { Tone::Warn }
}

/// A string at `key` of an object `v`, when non-empty.
pub(crate) fn str_at<'a>(v: JsonRef<'a>, key: &str) -> Option<&'a str> {
    v.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// Entries of the array at `key`; empty when absent.
pub(crate) fn arr_at<'a>(v: JsonRef<'a>, key: &str) -> Array<'a> {
    v.get(key)
        .and_then(JsonRef::as_array)
        .unwrap_or(Array::EMPTY)
}

/// `spec` of `res`, or null.
pub(crate) fn spec(res: &Resource) -> JsonRef<'_> {
    res.json().get("spec").unwrap_or(JsonRef::NULL)
}

/// `status` of `res`, or null.
pub(crate) fn status(res: &Resource) -> JsonRef<'_> {
    res.json().get("status").unwrap_or(JsonRef::NULL)
}

/// Joins `items` with commas, dropping any beyond `max` for a `+N more` suffix (kubectl's
/// convention for endpoint lists).
pub(crate) fn join_capped(items: impl IntoIterator<Item = String>, max: usize) -> String {
    let mut out = String::new();
    let mut extra = 0usize;
    for (i, item) in items.into_iter().enumerate() {
        if i >= max {
            extra += 1;
            continue;
        }
        if i > 0 {
            out.push(',');
        }
        out.push_str(&item);
    }
    if extra > 0 {
        let _ = write!(out, " + {extra} more...");
    }
    out
}

/// `k=v,k=v` of a JSON object of strings, in key order; empty for anything else.
pub(crate) fn selector_text(v: Option<JsonRef<'_>>) -> String {
    let Some(map) = v.and_then(JsonRef::as_object) else {
        return String::new();
    };
    key_values(map.iter().map(|(k, v)| (k, v.as_str().unwrap_or_default())))
}

/// Every container image of a pod template `spec`, comma-separated.
pub(crate) fn images_text(pod_spec: JsonRef<'_>) -> String {
    let mut out = String::new();
    for c in arr_at(pod_spec, "containers") {
        if let Some(image) = str_at(c, "image") {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(image);
        }
    }
    out
}
