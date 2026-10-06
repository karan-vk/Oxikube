//! Labels and annotations: copy, expand, and virtualisation of long lists.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use serde_json::{Value, json};

use super::fixture::{Detail, edited, pod_ref, web_pod};
use crate::table::tests::fixture::cluster;

#[gpui::test]
fn copying_a_label_is_a_command_that_writes_key_equals_value(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.open(&pod_ref("web-0"));
    d.f.dispatcher.clear();
    d.click("detail-copy-label-1");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceCopyLabel {
            target: pod_ref("web-0"),
            key: "tier".into(),
            annotation: false,
        }],
        "the value does not travel in the command"
    );
    let copied =
        d.f.vcx
            .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("tier=frontend"));

    d.click("detail-copy-annotation-0");
    let copied =
        d.f.vcx
            .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied.as_deref(), Some("note=hello"));
    let _ = cluster();
}

#[gpui::test]
fn a_long_annotation_is_cut_until_expanded(cx: &mut TestAppContext) {
    let long = "x".repeat(500);
    let pod = edited(web_pod(), |json| {
        json["metadata"]["annotations"]["big"] = json!(long);
    });
    let mut d = Detail::new(cx, [pod]);
    let view = d.open(&pod_ref("web-0"));
    // "big" sorts before "note": annotation 0.
    assert!(
        d.shown("detail-expand-annotation-0"),
        "a long value offers More"
    );
    assert!(
        !d.shown("detail-expand-annotation-1"),
        "a short one does not"
    );
    assert!(!d.read(&view, |v| v.is_expanded(true, "big")));
    let entry = d
        .read(&view, |v| {
            v.model().and_then(|m| m.meta_entry("big", true).cloned())
        })
        .unwrap();
    assert!(entry.collapsed().chars().count() < 200);

    d.click("detail-expand-annotation-0");
    assert!(d.read(&view, |v| v.is_expanded(true, "big")));
    d.click("detail-expand-annotation-0");
    assert!(
        !d.read(&view, |v| v.is_expanded(true, "big")),
        "and Less folds it again"
    );
    // The copy action still copies the whole value.
    assert_eq!(
        d.read(&view, |v| v.copy_text("big", true)),
        Some(format!("big={long}"))
    );
}

#[gpui::test]
fn a_long_label_list_builds_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let many = edited(web_pod(), |json| {
        let labels: serde_json::Map<String, Value> = (0..5_000)
            .map(|i| (format!("label-{i:05}"), json!(format!("value-{i}"))))
            .collect();
        json["metadata"]["labels"] = Value::Object(labels);
    });
    let mut d = Detail::new(cx, [many]);
    let view = d.open(&pod_ref("web-0"));
    d.draw();
    let (rows, built) = d.read(&view, |v| (v.rows().len(), v.rendered_rows));
    assert!(rows > 5_000, "every label is a row of the list: {rows}");
    assert!(built > 0, "something was drawn");
    assert!(
        built < 200,
        "{built} rows built for {rows}: the list is not virtualised"
    );
}
