//! What leaves the events view: a row as a line of text or as JSON for the clipboard
//! (`events::CopyRow`, `events::CopyRowJson`), and the listed events as a JSON file
//! (`events::ExportJson`).
//!
//! Every message goes through the redaction module on the way out (`oxikube_domain::redact`):
//! events rarely hold secrets, but a message is whatever a controller wrote, and a token pasted
//! into a ticket is a leak. The JSON is the domain [`Event`]'s own shape (both API versions
//! merged), so an export reads back with `serde_json`.

use std::borrow::Cow;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::event::Event;
use oxikube_domain::redact::redact;

use crate::model::{object_label, type_label};

/// `event` with its message redacted (borrowed when nothing needed it).
fn redacted(event: &Event) -> Cow<'_, Event> {
    match redact(&event.message) {
        Cow::Borrowed(_) => Cow::Borrowed(event),
        Cow::Owned(message) => Cow::Owned(Event {
            message,
            ..event.clone()
        }),
    }
}

/// One row as one line, tab separated: last seen (RFC 3339), type, reason, namespace,
/// `Kind/name`, source, count, message (redacted, its line breaks as spaces).
pub fn row_text(event: &Event) -> String {
    let event = redacted(event);
    let time = event
        .last_seen
        .or(event.first_seen)
        .map_or_else(|| "-".to_owned(), |t: Timestamp| t.to_string());
    let message: String = event
        .message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    [
        time,
        type_label(event.event_type).to_owned(),
        event.reason.to_string(),
        event.namespace.as_deref().unwrap_or("-").to_owned(),
        object_label(&event),
        event
            .reporting_component
            .as_deref()
            .unwrap_or("-")
            .to_owned(),
        event.count.to_string(),
        message,
    ]
    .join("\t")
}

/// One event as pretty JSON (the message redacted).
pub fn row_json(event: &Event) -> String {
    serde_json::to_string_pretty(&*redacted(event)).unwrap_or_default()
}

/// `events` as a pretty JSON array (the messages redacted), in the order given: what
/// `events::ExportJson` writes.
pub fn export_json(events: &[Arc<Event>]) -> Vec<u8> {
    let redacted: Vec<Cow<'_, Event>> = events.iter().map(|e| redacted(e)).collect();
    let mut out = serde_json::to_vec_pretty(&redacted).unwrap_or_default();
    out.push(b'\n');
    out
}

/// The file name the save panel suggests: `events-<context>-<time>.json`, safe on every OS.
pub fn suggested_file_name(context: &str, now: Timestamp) -> String {
    let context: String = context
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = now.strftime("%Y%m%d-%H%M%S");
    format!("events-{context}-{stamp}.json")
}

#[cfg(test)]
mod tests {
    use oxikube_testkit::event;

    use super::*;

    fn backoff() -> Event {
        event()
            .warning()
            .reason("BackOff")
            .message("Back-off pulling image\n\"registry/app\"")
            .count(4)
            .build()
    }

    #[test]
    fn a_row_is_one_tab_separated_line() {
        let text = row_text(&backoff());
        assert!(!text.contains('\n'));
        let fields: Vec<&str> = text.split('\t').collect();
        assert_eq!(
            fields,
            [
                "2026-01-01T00:00:00Z",
                "Warning",
                "BackOff",
                "default",
                "Pod/web-0",
                "default-scheduler",
                "4",
                "Back-off pulling image \"registry/app\"",
            ]
        );
    }

    #[test]
    fn json_reads_back_as_the_event() {
        let e = backoff();
        let back: Event = serde_json::from_str(&row_json(&e)).unwrap();
        assert_eq!(back, e);
        let all: Vec<Event> =
            serde_json::from_slice(&export_json(&[Arc::new(e.clone()), Arc::new(e)])).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn secrets_in_a_message_are_redacted_in_every_form() {
        let leak = event()
            .message("failed: Authorization: Bearer abc.def.ghi-secret-token")
            .build();
        for out in [
            row_text(&leak),
            row_json(&leak),
            String::from_utf8(export_json(&[Arc::new(leak.clone())])).unwrap(),
        ] {
            assert!(!out.contains("abc.def.ghi-secret-token"), "{out}");
        }
    }

    #[test]
    fn the_suggested_name_is_safe() {
        let now: Timestamp = "2026-03-04T05:06:07Z".parse().unwrap();
        assert_eq!(
            suggested_file_name("arn:aws/eks prod", now),
            "events-arn_aws_eks_prod-20260304-050607.json"
        );
    }
}
