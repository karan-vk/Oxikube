//! The keymap editor's rows over the installed keymap (E21-S07): what the loader rejects is not
//! listed and replaces nothing, and contexts are spelled as the predicate spells them.

mod common;

use gpui::TestAppContext;
use oxikube_keymap::{
    KeymapLayer, KeymapOptions, KeymapPlatform, KeymapRow, RowStatus, init_with_text, keymap_rows,
};

fn rows_for(cx: &mut TestAppContext, user: &str) -> Vec<KeymapRow> {
    cx.update(|cx| {
        let options = KeymapOptions {
            platform: KeymapPlatform::MacOs,
            vim: false,
        };
        init_with_text(user, options, cx);
        keymap_rows(cx)
    })
}

fn bound<'a>(rows: &'a [KeymapRow], action: &str) -> Vec<&'a KeymapRow> {
    rows.iter()
        .filter(|row| row.action == action && row.source.is_some())
        .collect()
}

#[gpui::test]
fn bindings_the_loader_rejects_are_not_listed_and_replace_nothing(cx: &mut TestAppContext) {
    let rows = rows_for(
        cx,
        r#"[
          {"bindings": {"cmd-shift-p": "kmtest::Missing", "cmd-j": ["kmtest::Scale", {"replicas": "many"}]}},
          {"context": "Pane &&", "bindings": {"cmd-k": "kmtest::Alpha"}}
        ]"#,
    );
    let palette = rows
        .iter()
        .find(|row| row.action == "palette::Toggle" && row.keystrokes_text() == "cmd-shift-p")
        .expect("the default binding of the palette is listed");
    assert_eq!(
        palette.status,
        RowStatus::Active,
        "an unknown action replaces nothing"
    );
    assert_eq!(palette.source, Some(KeymapLayer::Default));
    assert!(bound(&rows, "kmtest::Missing").is_empty(), "unknown action");
    assert!(bound(&rows, "kmtest::Scale").is_empty(), "invalid data");
    assert!(bound(&rows, "kmtest::Alpha").is_empty(), "invalid context");
}

#[gpui::test]
fn a_context_is_spelled_as_the_predicate_spells_it(cx: &mut TestAppContext) {
    let rows = rows_for(
        cx,
        r#"[{"context": "Pane&&!Editor", "bindings": {"cmd-l": "kmtest::Beta"}}]"#,
    );
    let beta = bound(&rows, "kmtest::Beta");
    assert_eq!(beta.len(), 1);
    assert_eq!(beta[0].context.as_deref(), Some("Pane && !Editor"));
    assert_eq!(beta[0].source, Some(KeymapLayer::User));
    assert_eq!(beta[0].status, RowStatus::Active);
}
