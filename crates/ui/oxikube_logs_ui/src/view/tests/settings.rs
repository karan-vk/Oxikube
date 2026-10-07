//! The `logs` settings in the view: they are its first options, and a change of them applies to
//! the open view without reopening the stream.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_settings::SettingsStore;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, cluster, lines};
use crate::view::TAIL_LINES;

fn set_user(fx: &mut Fx, text: &str) {
    let text = text.to_owned();
    fx.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(&text).expect("valid settings");
        });
    });
    fx.settle();
}

/// A window whose settings store holds `user` before any view is opened.
fn fx_with(cx: &mut TestAppContext, user: &str) -> Fx {
    let mut fx = Fx::new(cx);
    fx.vcx.update(|_, cx| {
        let mut store =
            SettingsStore::new(oxikube_assets::default_settings()).expect("the embedded defaults");
        store.set_user_settings(user).expect("valid settings");
        cx.set_global(store);
    });
    fx
}

fn timeline() -> Timeline<oxikube_domain::log::LogLine> {
    Timeline::immediate(lines(0, 5)).keep_open()
}

#[gpui::test]
fn without_settings_a_view_starts_as_s02_shipped_it(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(timeline());
    let options = fx.read(&view, |v| v.options().clone());
    assert!(!options.wrap && !options.timestamps);
    assert_eq!(fx.opened()[0].tail_lines, Some(TAIL_LINES));
}

#[gpui::test]
fn the_settings_are_the_first_options_of_a_view(cx: &mut TestAppContext) {
    let mut fx = fx_with(
        cx,
        r#"{ "logs": { "wrap": true, "timestamps": true, "default_tail": 200,
                       "json_auto_detect": false } }"#,
    );
    let view = fx.open(timeline());
    let options = fx.read(&view, |v| v.options().clone());
    assert!(options.wrap && options.timestamps && !options.json);
    assert_eq!(
        fx.opened()[0].tail_lines,
        Some(200),
        "the tail reads default_tail lines"
    );
    let text = fx.read(&view, |v| v.row_text(0)).unwrap();
    assert!(text.starts_with("2026-10-04T"), "timestamps are on: {text}");

    // Picking the tail range again reads default_tail too, not the built-in 1 000.
    fx.script(timeline());
    fx.script(timeline());
    fx.keys("2");
    fx.keys("0");
    assert_eq!(fx.opened().last().unwrap().tail_lines, Some(200));
}

#[gpui::test]
fn a_cluster_can_override_what_the_view_starts_with(cx: &mut TestAppContext) {
    let user = format!(
        r#"{{ "clusters": {{ "{}": {{ "logs": {{ "wrap": true, "default_tail": 50 }} }} }} }}"#,
        cluster().as_str()
    );
    let mut fx = fx_with(cx, &user);
    let view = fx.open(timeline());
    assert!(fx.read(&view, |v| v.options().wrap), "the cluster's wrap");
    assert_eq!(fx.opened()[0].tail_lines, Some(50));
}

#[gpui::test]
fn wrap_timestamps_and_json_apply_live_without_reopening(cx: &mut TestAppContext) {
    let mut fx = fx_with(cx, "{}");
    let view = fx.open(timeline());
    assert_eq!(fx.opened().len(), 1);

    set_user(&mut fx, r#"{ "logs": { "wrap": true } }"#);
    assert!(fx.read(&view, |v| v.options().wrap));
    set_user(
        &mut fx,
        r#"{ "logs": { "wrap": true, "timestamps": true, "json_auto_detect": false } }"#,
    );
    let options = fx.read(&view, |v| v.options().clone());
    assert!(options.wrap && options.timestamps && !options.json);
    let text = fx.read(&view, |v| v.row_text(0)).unwrap();
    assert!(text.starts_with("2026-10-04T"), "{text}");

    set_user(&mut fx, "{}");
    let options = fx.read(&view, |v| v.options().clone());
    assert!(!options.wrap && !options.timestamps && options.json);

    assert_eq!(fx.opened().len(), 1, "no setting reopened the stream");
    assert_eq!(
        fx.read(&view, |v| v.line_window().line_count()),
        5,
        "the lines stayed"
    );
}

#[gpui::test]
fn a_cluster_override_applies_live_to_that_clusters_views(cx: &mut TestAppContext) {
    let mut fx = fx_with(cx, "{}");
    let view = fx.open(timeline());
    let user = format!(
        r#"{{ "clusters": {{ "{}": {{ "logs": {{ "timestamps": true }} }} }} }}"#,
        cluster().as_str()
    );
    set_user(&mut fx, &user);
    assert!(fx.read(&view, |v| v.options().timestamps));
}

#[gpui::test]
fn default_tail_waits_for_the_next_read_and_never_reopens(cx: &mut TestAppContext) {
    let mut fx = fx_with(cx, "{}");
    let view = fx.open(timeline());
    set_user(&mut fx, r#"{ "logs": { "default_tail": 10 } }"#);
    assert_eq!(fx.opened().len(), 1, "what is being read is left alone");
    assert_eq!(fx.read(&view, |v| v.options().default_tail), 10);
}

#[gpui::test]
fn a_change_redraws_the_view_once_and_an_unrelated_one_not_at_all(cx: &mut TestAppContext) {
    let mut fx = fx_with(cx, "{}");
    let view = fx.open(timeline());
    let redraws = Rc::new(Cell::new(0));
    fx.vcx.update(|_, cx| {
        let redraws = redraws.clone();
        cx.observe(&view, move |_, _| redraws.set(redraws.get() + 1))
            .detach();
    });

    // Wrap, timestamps and JSON detection change in one edit: one redraw.
    set_user(
        &mut fx,
        r#"{ "logs": { "wrap": true, "timestamps": true, "json_auto_detect": false } }"#,
    );
    assert_eq!(redraws.get(), 1, "one edit, one redraw");

    // The buffer size is the service's business; the view is not touched.
    redraws.set(0);
    set_user(
        &mut fx,
        r#"{ "logs": { "wrap": true, "timestamps": true, "json_auto_detect": false,
                       "buffer_lines": 9000 } }"#,
    );
    assert_eq!(redraws.get(), 0);
    // Nor by an edit of some other setting.
    set_user(
        &mut fx,
        r#"{ "logs": { "wrap": true, "timestamps": true, "json_auto_detect": false },
             "ui_scale": 1.25 }"#,
    );
    assert_eq!(redraws.get(), 0);
}
