//! Zoom: steps, clamping, the rem size and `u(px)`, persistence, hot reload.

use gpui::{TestAppContext, px};
use oxikube_settings::{Settings as _, update_user_settings};
use oxikube_ui::{UiScale, u};

use super::{open_window, rem_size, settings_text, setup, setup_without_settings};
use crate::session::{
    SessionSettings, ZoomIn, ZoomOut, ZoomReset,
    zoom::{ZOOM_STEPS, zoom_in_from, zoom_out_from},
};

fn scale(cx: &mut TestAppContext) -> f32 {
    cx.update(|cx| UiScale::get(cx).factor())
}

#[test]
fn steps_are_sorted_and_span_the_supported_range() {
    assert!(ZOOM_STEPS.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ZOOM_STEPS[0], UiScale::MIN);
    assert_eq!(ZOOM_STEPS[ZOOM_STEPS.len() - 1], UiScale::MAX);
    assert!(ZOOM_STEPS.contains(&1.0), "100 % is a step");
}

#[test]
fn zoom_steps_move_one_level_and_clamp_at_the_ends() {
    let at = |factor| UiScale::new(factor);
    assert_eq!(zoom_in_from(at(1.0)).factor(), 1.1);
    assert_eq!(zoom_out_from(at(1.0)).factor(), 0.9);
    assert_eq!(zoom_in_from(at(UiScale::MAX)).factor(), UiScale::MAX);
    assert_eq!(zoom_out_from(at(UiScale::MIN)).factor(), UiScale::MIN);
    // Off the ladder (a hand-edited setting): the nearest level in that direction.
    assert_eq!(zoom_in_from(at(1.15)).factor(), 1.2);
    assert_eq!(zoom_out_from(at(1.15)).factor(), 1.1);
    // Walking all the way up and down visits only ladder levels and ends at the limits.
    let mut zoom = at(UiScale::MIN);
    for _ in 0..40 {
        zoom = zoom_in_from(zoom);
    }
    assert_eq!(zoom.factor(), UiScale::MAX);
    for _ in 0..40 {
        zoom = zoom_out_from(zoom);
    }
    assert_eq!(zoom.factor(), UiScale::MIN);
}

#[gpui::test]
fn zoom_actions_change_the_rem_size_and_u_px(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_handle, mut vcx) = open_window(cx);
    let base = rem_size(&mut vcx);
    assert_eq!(u(px(100.)), px(100.), "100 % to start with");

    vcx.dispatch_action(ZoomIn);
    let zoomed = rem_size(&mut vcx);
    assert!(
        (zoomed - base * 1.1).abs() < 1e-3,
        "rem size {zoomed} is not 110 % of {base}"
    );
    assert!((f32::from(u(px(100.))) - 110.).abs() < 1e-3);

    vcx.dispatch_action(ZoomOut);
    vcx.dispatch_action(ZoomOut);
    let smaller = rem_size(&mut vcx);
    assert!((smaller - base * 0.9).abs() < 1e-3, "{smaller} vs {base}");
    assert!((f32::from(u(px(100.))) - 90.).abs() < 1e-3);

    vcx.dispatch_action(ZoomReset);
    assert!((rem_size(&mut vcx) - base).abs() < 1e-3);
    assert_eq!(u(px(100.)), px(100.));
}

#[gpui::test]
fn zoom_applies_within_the_next_frame_with_no_stale_size(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_handle, mut vcx) = open_window(cx);
    let base = rem_size(&mut vcx);
    // No `run_until_parked` between the keystroke and the frame: the setting file has not been
    // written yet, and the frame must still be at the new zoom.
    for expected in [1.1, 1.2, 1.3] {
        vcx.dispatch_action(ZoomIn);
        assert_eq!(scale(cx), expected, "applied at once");
        let rem = rem_size(&mut vcx);
        assert!((rem - base * expected).abs() < 1e-3, "{rem} at {expected}");
    }
    // The writes land afterwards and must not pull the zoom back to an older value.
    vcx.run_until_parked();
    assert_eq!(scale(cx), 1.3);
    assert!((rem_size(&mut vcx) - base * 1.3).abs() < 1e-3);
}

#[gpui::test]
fn zoom_is_persisted_and_clamped(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let (_handle, mut vcx) = open_window(cx);
    vcx.dispatch_action(ZoomIn);
    vcx.run_until_parked();
    assert!(
        settings_text(dir.path()).contains("\"ui_scale\": 1.1"),
        "settings.json: {}",
        settings_text(dir.path())
    );
    assert_eq!(
        cx.update(|cx| SessionSettings::get_global(cx).ui_scale.factor()),
        1.1
    );

    // A new run reads the file back and starts zoomed.
    cx.update(|cx| {
        oxikube_ui::set_ui_scale(cx, UiScale::IDENTITY);
        oxikube_settings::init_with_dir(dir.path(), cx);
        crate::session::settings::apply_and_observe(cx);
    });
    assert_eq!(scale(cx), 1.1, "restored from settings.json");

    // Zooming past the end stays at the end.
    for _ in 0..40 {
        vcx.dispatch_action(ZoomIn);
    }
    vcx.run_until_parked();
    assert_eq!(scale(cx), UiScale::MAX);
    assert!(settings_text(dir.path()).contains("\"ui_scale\": 3"));

    // And a file value out of range is clamped when read.
    cx.update(|cx| {
        update_user_settings::<SessionSettings>(cx, None, |c| c.ui_scale = Some(10.0)).detach();
    });
    vcx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            SessionSettings::get_global(cx).ui_scale.factor(),
            UiScale::MAX
        );
    });
    cx.update(|cx| {
        update_user_settings::<SessionSettings>(cx, None, |c| c.ui_scale = Some(0.01)).detach();
    });
    vcx.run_until_parked();
    assert_eq!(scale(cx), UiScale::MIN);
}

#[gpui::test]
fn editing_ui_scale_in_settings_zooms_through_hot_reload(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let (_handle, mut vcx) = open_window(cx);
    let base = rem_size(&mut vcx);
    cx.update(|cx| {
        update_user_settings::<SessionSettings>(cx, None, |c| c.ui_scale = Some(2.0)).detach();
    });
    vcx.run_until_parked();
    assert_eq!(scale(cx), 2.0);
    assert!((rem_size(&mut vcx) - base * 2.0).abs() < 1e-3);
}

#[gpui::test]
fn zoom_works_without_a_settings_store(cx: &mut TestAppContext) {
    setup_without_settings(cx);
    let (_handle, mut vcx) = open_window(cx);
    vcx.dispatch_action(ZoomIn);
    vcx.run_until_parked();
    assert_eq!(scale(cx), 1.1);
    vcx.dispatch_action(ZoomReset);
    assert_eq!(scale(cx), 1.0);
}

#[gpui::test]
fn a_settings_store_installed_later_is_picked_up(cx: &mut TestAppContext) {
    setup_without_settings(cx);
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("settings.json"), "{ \"ui_scale\": 1.5 }").unwrap();
    cx.update(|cx| oxikube_settings::init_with_dir(dir.path(), cx));
    cx.run_until_parked();
    assert_eq!(scale(cx), 1.5);
}
