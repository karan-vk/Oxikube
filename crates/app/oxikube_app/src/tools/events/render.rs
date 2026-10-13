//! One event as a line of text and as a JSON row: what an agent reads.
//!
//! Everything that came from the cluster (reason, message, names, the reporter) passes through
//! `oxikube_domain::redact` first: event messages quote image names with registry credentials in
//! the URL and, now and then, environment values. Messages are one line, cut to a character
//! budget on a char boundary.

use std::borrow::Cow;

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::redact::redact;
use serde_json::{Value, json};

/// `Normal`, `Warning` or `Other`.
pub(super) fn type_text(event: &Event) -> &'static str {
    match event.event_type {
        EventType::Normal => "Normal",
        EventType::Warning => "Warning",
        EventType::Other => "Other",
    }
}

/// `Pod/shop/web-0`, or `Node/worker-1` for a cluster-scoped object.
pub(super) fn object_text(event: &Event) -> String {
    let regarding = &event.regarding;
    match &regarding.namespace {
        Some(ns) => format!("{}/{ns}/{}", regarding.gvk.kind, regarding.name),
        None => format!("{}/{}", regarding.gvk.kind, regarding.name),
    }
}

/// `text` redacted, on one line, and at most `max` characters (a longer one ends in `…`).
pub(super) fn one_line(text: &str, max: usize) -> String {
    let redacted: Cow<'_, str> = redact(text);
    let mut out = String::with_capacity(redacted.len().min(max * 4));
    for (chars, c) in redacted.chars().enumerate() {
        if chars == max {
            out.push('…');
            return out;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    out
}

/// `45s`, `7m`, `2h`, `3d`: the coarsest whole unit of `age`.
pub(super) fn age_text(age: SignedDuration) -> String {
    let s = age.as_secs().max(0);
    match s {
        ..120 => format!("{s}s"),
        ..7_200 => format!("{}m", s / 60),
        ..172_800 => format!("{}h", s / 3_600),
        _ => format!("{}d", s / 86_400),
    }
}

/// When the event was last seen, else first seen.
fn seen(event: &Event) -> Option<Timestamp> {
    event.last_seen.or(event.first_seen)
}

fn reporter(event: &Event) -> Option<String> {
    event
        .reporting_component
        .as_deref()
        .map(|c| one_line(c, 64))
}

/// A row of `k8s.get_events`: stable keys, `null` for what the cluster did not say.
pub(super) fn json_row(event: &Event, now: Timestamp, max_message: usize) -> Value {
    json!({
        "type": type_text(event),
        "reason": one_line(&event.reason, 64),
        "object": one_line(&object_text(event), 160),
        "namespace": event.namespace.as_deref(),
        "count": event.count,
        "first_seen": event.first_seen.map(|t| t.to_string()),
        "last_seen": event.last_seen.map(|t| t.to_string()),
        "age": seen(event).map(|t| age_text(now.duration_since(t))),
        "message": one_line(&event.message, max_message),
        "reporter": reporter(event),
    })
}

/// The row as a tab-separated line: `TYPE REASON OBJECT COUNT AGE MESSAGE`.
pub(super) fn tsv_line(event: &Event, now: Timestamp, max_message: usize) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        type_text(event),
        one_line(&event.reason, 64),
        one_line(&object_text(event), 160),
        event.count,
        seen(event).map_or_else(|| "-".to_owned(), |t| age_text(now.duration_since(t))),
        one_line(&event.message, max_message),
    )
}

/// The column header of [`tsv_line`].
pub(super) const TSV_HEADER: &str = "TYPE\tREASON\tOBJECT\tCOUNT\tAGE\tMESSAGE";

/// The row as a log-style line for a context block:
/// `2026-01-01T00:00:00Z Warning BackOff x3 Pod/shop/web-0: message`.
pub(super) fn block_line(event: &Event, max_message: usize) -> String {
    format!(
        "{} {} {} x{} {}: {}",
        seen(event).map_or_else(|| "-".to_owned(), |t| t.to_string()),
        type_text(event),
        one_line(&event.reason, 64),
        event.count,
        one_line(&object_text(event), 160),
        one_line(&event.message, max_message),
    )
}
