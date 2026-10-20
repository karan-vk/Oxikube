//! k9s forward annotations in the start dialog (E15-S07): with `portforward.annotations` on, an
//! annotated pod shows "Suggested from annotations", a click fills the fields, an auto entry is
//! preselected, a taken local port warns, and nothing is sent until Forward. A plain pod or the
//! setting off shows no section.

use gpui::TestAppContext;
use oxikube_app::ForwardConfig;
use serde_json::json;

use super::fixture::Fixture;
use super::p;
use super::view_forward::{click, dialog, open_dialog, shown};
use crate::detail::tests::fixture::edited;

fn annotated_pod() -> oxikube_domain::Resource {
    edited(p("x", "apple-1", "1"), |json| {
        json["metadata"]["annotations"] = json!({
            "k9scli.io/port-forwards": "app::18080:8080, app::19090:9090",
            "k9scli.io/auto-port-forwards": "app::18080:8080",
        });
        json["spec"]["containers"] = json!([
            {"name": "app", "image": "app:1", "ports": [{"name": "http", "containerPort": 8080}]}
        ]);
    })
}

fn open_with(
    cx: &mut TestAppContext,
    pod: oxikube_domain::Resource,
    on: bool,
) -> (Fixture, gpui::Entity<crate::table::ResourceTable>) {
    let mut f = Fixture::with_forward(cx);
    f.forward
        .as_ref()
        .unwrap()
        .manager
        .set_config(ForwardConfig {
            annotations: on,
            ..Default::default()
        });
    f.connect_with([pod, p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    assert_eq!(f.selected(&table), ["apple-1"]);
    f.dispatcher.clear();
    (f, table)
}

#[gpui::test]
fn an_annotated_pod_lists_suggestions_preselects_the_auto_one_and_a_click_fills_the_fields(
    cx: &mut TestAppContext,
) {
    let (mut f, table) = open_with(cx, annotated_pod(), true);
    let d = open_dialog(&mut f, &table);
    f.settle();
    assert!(shown(&mut f, "pf-suggestions"));
    assert!(shown(&mut f, "pf-suggestion-0") && shown(&mut f, "pf-suggestion-1"));
    let rows = f.vcx.update(|_, cx| {
        d.read(cx)
            .suggestions()
            .iter()
            .map(|s| (s.label(), s.forward.auto))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        rows,
        [
            ("app 18080:8080".to_owned(), true),
            ("app 19090:9090".to_owned(), false)
        ]
    );
    // The auto entry is preselected: fixed local port, remote port.
    let input = f.vcx.update(|_, cx| d.read(cx).input(cx));
    assert_eq!(
        (input.remote_port.as_str(), input.local_port.as_str()),
        ("8080", "18080")
    );
    assert!(!input.local_auto);

    click(&mut f, "pf-suggestion-1");
    let input = f.vcx.update(|_, cx| d.read(cx).input(cx));
    assert_eq!(
        (input.remote_port.as_str(), input.local_port.as_str()),
        ("9090", "19090")
    );
    assert!(
        f.forward.as_ref().unwrap().calls.lock().is_empty() && f.dispatcher.sent().is_empty(),
        "a suggestion only fills fields"
    );
}

#[gpui::test]
fn a_suggestion_whose_local_port_is_taken_warns_and_is_not_preselected(cx: &mut TestAppContext) {
    // The conflict itself is computed (and tested) in `oxikube_app`; here the dialog shows it.
    let (mut f, table) = open_with(cx, annotated_pod(), true);
    let d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| {
            let mut suggestions = d.suggestions().to_vec();
            suggestions[0].conflict = Some("Local port 18080 is already used by db".into());
            d.fill("", "", "127.0.0.1", window, cx);
            d.set_suggestions(suggestions, window, cx);
        })
    });
    f.settle();
    assert!(shown(&mut f, "pf-suggestion-conflict-0"));
    assert!(!shown(&mut f, "pf-suggestion-conflict-1"));
    let input = f.vcx.update(|_, cx| d.read(cx).input(cx));
    assert!(
        input.local_port.is_empty(),
        "a conflicting entry is not preselected"
    );
}

#[gpui::test]
fn a_plain_pod_or_the_setting_off_shows_no_suggestions(cx: &mut TestAppContext) {
    let plain = edited(p("x", "apple-1", "1"), |json| {
        json["spec"]["containers"] =
            json!([{"name": "app", "image": "a", "ports": [{"containerPort": 8080}]}]);
    });
    let (mut f, table) = open_with(cx, plain, true);
    open_dialog(&mut f, &table);
    f.settle();
    assert!(!shown(&mut f, "pf-suggestions"));
    assert!(shown(&mut f, "pf-port-0"), "the ports are still read");
}

#[gpui::test]
fn annotations_are_ignored_while_the_setting_is_off(cx: &mut TestAppContext) {
    let (mut f, table) = open_with(cx, annotated_pod(), false);
    let d = open_dialog(&mut f, &table);
    f.settle();
    assert!(!shown(&mut f, "pf-suggestions"));
    assert!(dialog(&mut f).is_some());
    let input = f.vcx.update(|_, cx| d.read(cx).input(cx));
    assert_eq!(input.remote_port, "8080");
    assert!(input.local_auto, "nothing was preselected");
}
