//! E10-S05: hover and completion never panic and never point outside the text, whatever the
//! buffer and the cursor.

#[path = "assist/common.rs"]
mod common;

use common::*;
use oxikube_editor::assist::{Buffer, complete_at, hover_at};
use oxikube_editor::yaml::parse;
use proptest::prelude::*;

/// A buffer of the characters YAML cares about, led by a kind so a schema applies.
fn buffer() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9:\\- \\n#|>{}\\[\\]\"'é.]{0,120}"
        .prop_map(|tail| format!("apiVersion: v1\nkind: Pod\n{tail}"))
}

fn boundary(text: &str, at: usize) -> usize {
    let mut at = at % (text.len() + 1);
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

proptest! {
    #[test]
    fn completion_ranges_stay_inside_the_text(text in buffer(), at in 0usize..400) {
        let at = boundary(&text, at);
        let items = complete_at(&Buffer::new(text.as_str()), schemas, at);
        for item in &items {
            prop_assert!(item.replace.start <= item.replace.end && item.replace.end <= text.len());
            prop_assert!(text.is_char_boundary(item.replace.start));
            prop_assert!(text.is_char_boundary(item.replace.end));
            prop_assert!(item.replace.start <= at, "{:?} after the cursor at {at}", item.replace);
            prop_assert!(!item.label.is_empty());
        }
    }

    #[test]
    fn hover_ranges_stay_inside_the_text_and_contain_the_pointer(text in buffer(), at in 0usize..400) {
        let at = boundary(&text, at);
        let parsed = parse(&text);
        if let Some(doc) = hover_at(&parsed, schemas, at) {
            prop_assert!(doc.range.start <= at && at < doc.range.end, "{:?} at {at}", doc.range);
            prop_assert!(doc.range.end <= text.len());
            prop_assert!(!doc.to_markdown().is_empty());
        }
    }
}
