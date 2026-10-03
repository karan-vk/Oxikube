//! [`ContextBlock`]: a bounded piece of cluster context handed to an agent.
//!
//! Context providers resolve mentions such as `@pod/default/web-0`,
//! `@logs/...` or `@events/...` into blocks (the glossary's ContentBlock). A
//! block is plain data: a title, a MIME type and a text body.
//!
//! # Size budget
//!
//! Agents have finite context, so a body is capped at
//! [`MAX_CONTEXT_BLOCK_BYTES`]. [`ContextBlock::bounded`] cuts at a char
//! boundary and sets [`ContextBlock::truncated`] so a provider can report the
//! cut to the user ("logs truncated to 64 KiB"). The flag survives serde, and
//! deserialising re-applies the caps.
//!
//! The domain never redacts. A context provider must redact Secret data and
//! tokens *before* building a block (non-negotiable 5); a block may be written
//! to an agent thread that is stored locally.

use serde::{Deserialize, Serialize};

use crate::bounds::truncate_in_place;

/// Longest block body kept, in bytes (64 KiB).
pub const MAX_CONTEXT_BLOCK_BYTES: usize = 64 * 1024;

/// Longest title kept, in bytes.
pub const MAX_CONTEXT_TITLE_BYTES: usize = 256;

/// Longest MIME type kept, in bytes (RFC 6838 caps a type/subtype at 127 each; this is generous).
pub const MAX_CONTEXT_MIME_BYTES: usize = 255;

/// A titled, typed, size-bounded text block of agent context.
///
/// Field names are stable: the type is stored in agent threads and sent to agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ContextBlockWire")]
pub struct ContextBlock {
    /// Short label, for example `Pod default/web-0`.
    pub title: String,
    /// MIME type of `body`, for example `application/yaml` or `text/plain`.
    pub mime: String,
    /// The content, at most [`MAX_CONTEXT_BLOCK_BYTES`] long.
    pub body: String,
    /// Whether any of the fields was cut to fit its bound.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

/// Deserialisation shape; converting through it re-applies the caps.
#[derive(Deserialize)]
struct ContextBlockWire {
    title: String,
    mime: String,
    body: String,
    #[serde(default)]
    truncated: bool,
}

impl From<ContextBlockWire> for ContextBlock {
    fn from(w: ContextBlockWire) -> Self {
        let mut block = ContextBlock::bounded(w.title, w.mime, w.body);
        block.truncated |= w.truncated;
        block
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ContextBlock {
    /// Build a block, cutting the body to [`MAX_CONTEXT_BLOCK_BYTES`], the title
    /// to [`MAX_CONTEXT_TITLE_BYTES`] and the MIME type to [`MAX_CONTEXT_MIME_BYTES`],
    /// each on a char boundary. [`truncated`](Self::truncated) is set when any cut happened.
    pub fn bounded(
        title: impl Into<String>,
        mime: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        let (mut title, mut mime, mut body) = (title.into(), mime.into(), body.into());
        let mut truncated = truncate_in_place(&mut title, MAX_CONTEXT_TITLE_BYTES);
        truncated |= truncate_in_place(&mut mime, MAX_CONTEXT_MIME_BYTES);
        truncated |= truncate_in_place(&mut body, MAX_CONTEXT_BLOCK_BYTES);
        Self {
            title,
            mime,
            body,
            truncated,
        }
    }

    /// A `text/plain` block.
    pub fn text(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::bounded(title, "text/plain", body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn within_budget_is_untouched() {
        let b = ContextBlock::bounded("Pod web-0", "application/yaml", "kind: Pod\n");
        assert_eq!(b.body, "kind: Pod\n");
        assert!(!b.truncated);
        assert_eq!(ContextBlock::text("t", "b").mime, "text/plain");
    }

    #[test]
    fn over_budget_is_cut_at_a_char_boundary() {
        let body = "é".repeat(MAX_CONTEXT_BLOCK_BYTES); // 2 bytes each, twice the budget
        let b = ContextBlock::bounded("t", "text/plain", body);
        assert!(b.truncated);
        assert_eq!(b.body.len(), MAX_CONTEXT_BLOCK_BYTES);
        assert!(b.body.chars().all(|c| c == 'é'));

        // A budget that lands mid-char backs off by one byte.
        let body = format!("x{}", "é".repeat(MAX_CONTEXT_BLOCK_BYTES));
        let b = ContextBlock::bounded("t", "text/plain", body);
        assert_eq!(b.body.len(), MAX_CONTEXT_BLOCK_BYTES - 1);
    }

    #[test]
    fn exact_budget_is_not_truncated() {
        let b = ContextBlock::text("t", "a".repeat(MAX_CONTEXT_BLOCK_BYTES));
        assert!(!b.truncated);
    }

    #[test]
    fn title_and_mime_are_bounded_too() {
        let b = ContextBlock::bounded("t".repeat(1000), "m".repeat(1000), "x");
        assert!(b.truncated);
        assert_eq!(b.title.len(), MAX_CONTEXT_TITLE_BYTES);
        assert_eq!(b.mime.len(), MAX_CONTEXT_MIME_BYTES);
    }

    #[test]
    fn serde_round_trip_keeps_the_truncation_flag() {
        let small = ContextBlock::text("t", "b");
        let json = serde_json::to_value(&small).unwrap();
        assert!(json.get("truncated").is_none());
        assert_eq!(serde_json::from_value::<ContextBlock>(json).unwrap(), small);

        let cut = ContextBlock::text("t", "a".repeat(MAX_CONTEXT_BLOCK_BYTES + 1));
        let json = serde_json::to_value(&cut).unwrap();
        assert_eq!(json["truncated"], true);
        let back: ContextBlock = serde_json::from_value(json).unwrap();
        assert!(back.truncated);
        assert_eq!(back, cut);
    }

    #[test]
    fn deserialising_re_applies_the_cap() {
        let json = serde_json::json!({
            "title": "t", "mime": "text/plain", "body": "a".repeat(MAX_CONTEXT_BLOCK_BYTES * 2)
        });
        let b: ContextBlock = serde_json::from_value(json).unwrap();
        assert_eq!(b.body.len(), MAX_CONTEXT_BLOCK_BYTES);
        assert!(b.truncated);
    }

    proptest! {
        #[test]
        fn arbitrary_utf8_never_panics(
            title in any::<String>(), mime in any::<String>(), body in any::<String>()
        ) {
            let b = ContextBlock::bounded(title.clone(), mime, body.clone());
            prop_assert!(b.body.len() <= MAX_CONTEXT_BLOCK_BYTES);
            prop_assert!(b.title.len() <= MAX_CONTEXT_TITLE_BYTES);
            prop_assert!(body.starts_with(&b.body));
            prop_assert!(title.starts_with(&b.title));
        }
    }
}
