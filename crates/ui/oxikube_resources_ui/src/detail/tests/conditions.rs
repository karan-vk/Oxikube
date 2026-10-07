//! The Overview's conditions as two lines, and label, annotation and status keys that are cut
//! with a tooltip instead of wrapping mid-word (E07-U559).

use gpui::{Bounds, Pixels, TestAppContext};
use serde_json::json;

use super::fixture::{Detail, edited, pod_ref, web_pod};

fn bounds(d: &mut Detail, selector: &'static str) -> Bounds<Pixels> {
    d.draw();
    d.f.vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"))
}

#[gpui::test]
fn a_condition_is_two_lines_and_its_type_never_wraps(cx: &mut TestAppContext) {
    let pod = edited(web_pod(), |json| {
        json["metadata"]["annotations"] = json!({
            "a.very.long.annotation.domain.example.com/with-a-long-key-name": "x"
        });
        json["status"]["conditions"] = json!([
            {"type": "PodReadyToStartContainers", "status": "True",
             "lastTransitionTime": "2026-01-01T00:00:10Z"},
            {"type": "ContainersReady", "status": "False", "reason": "ContainersNotReady",
             "message": "containers with unready status: [web sidecar] and a message that is long enough to wrap",
             "lastTransitionTime": "2026-01-01T00:00:05Z"}
        ]);
    });
    let mut d = Detail::new(cx, [pod]);
    let view = d.open(&pod_ref("web-0"));
    d.read(&view, |_| ());
    // Scroll the conditions into the window.
    d.update(&view, |v, cx| {
        let at = v
            .rows()
            .iter()
            .position(|r| matches!(r, crate::detail::model::Row::Condition(0)))
            .expect("a condition row");
        v.overview_list().scroll_to_reveal_item(at);
        cx.notify();
    });

    // A type with nothing else is one line: its status and age share it, no second line.
    let first = bounds(&mut d, "detail-condition-0");
    let kind = bounds(&mut d, "detail-condition-type-0");
    let status = bounds(&mut d, "detail-condition-status-0");
    let age = bounds(&mut d, "detail-condition-age-0");
    assert!(
        !d.shown("detail-condition-detail-0"),
        "no reason, no message: one line"
    );
    assert!(kind.size.height < first.size.height + gpui::px(1.));
    assert!(
        kind.size.height < gpui::px(30.),
        "the type is one line, not wrapped: {:?}",
        kind.size
    );
    assert!(
        kind.right() <= status.left() + gpui::px(1.),
        "type, then status"
    );
    assert!(status.right() <= age.left() + gpui::px(1.), "then age");

    // A condition with a reason and message: line one, then the muted detail below it.
    let second = bounds(&mut d, "detail-condition-1");
    let kind1 = bounds(&mut d, "detail-condition-type-1");
    let detail = bounds(&mut d, "detail-condition-detail-1");
    assert!(
        detail.top() >= kind1.bottom() - gpui::px(1.),
        "the detail is under line one"
    );
    assert!(
        detail.size.height > kind1.size.height,
        "and it wraps: the message is long"
    );
    assert!(second.contains(&detail.origin), "inside its row");
    // Statuses of different rows line up.
    let status1 = bounds(&mut d, "detail-condition-status-1");
    assert_eq!(status.left(), status1.left(), "statuses are one column");
    assert_eq!(
        age.right(),
        bounds(&mut d, "detail-condition-age-1").right()
    );
}

#[gpui::test]
fn long_label_and_annotation_keys_are_cut_to_one_line(cx: &mut TestAppContext) {
    let pod = edited(web_pod(), |json| {
        json["metadata"]["labels"]["app.kubernetes.io/a-very-long-label-key-name"] = json!("v");
        json["metadata"]["annotations"] = json!({
            "a.very.long.annotation.domain.example.com/with-a-long-key-name": "x"
        });
    });
    let mut d = Detail::new(cx, [pod]);
    let view = d.open(&pod_ref("web-0"));
    let model = d.read(&view, |v| v.model().cloned()).expect("a model");
    let long_label = model
        .labels
        .iter()
        .position(|l| l.key.starts_with("app.kubernetes.io/a-very"))
        .expect("listed");
    let label_selector: &'static str =
        Box::leak(format!("detail-label-key-{long_label}").into_boxed_str());

    // Every key cell has the one shared width and one line's height, however long the key: it is
    // cut, not wrapped.
    let short = bounds(&mut d, "detail-label-key-0");
    let long = bounds(&mut d, label_selector);
    let annotation = bounds(&mut d, "detail-annotation-key-0");
    assert_eq!(short.size.width, long.size.width, "one key column");
    assert_eq!(short.size.width, annotation.size.width);
    for (what, bounds) in [("label", long), ("annotation", annotation)] {
        assert_eq!(
            bounds.size.height, short.size.height,
            "a long {what} key is one line like a short one"
        );
    }
}
