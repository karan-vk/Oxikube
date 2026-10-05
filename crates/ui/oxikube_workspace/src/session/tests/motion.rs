//! Reduce-motion: the setting overrides the OS preference, and the result reaches GPUI.

use gpui::TestAppContext;
use oxikube_settings::update_user_settings;

use super::{setup, setup_without_settings};
use crate::session::{
    ReduceMotionSetting, SessionSettings, os_reduce_motion, resolve_reduce_motion,
    set_os_reduce_motion,
};

#[test]
fn the_override_beats_the_os_value() {
    for os in [false, true] {
        assert!(resolve_reduce_motion(ReduceMotionSetting::On, os));
        assert!(!resolve_reduce_motion(ReduceMotionSetting::Off, os));
        assert_eq!(resolve_reduce_motion(ReduceMotionSetting::System, os), os);
    }
}

fn set_setting(cx: &mut TestAppContext, value: ReduceMotionSetting) {
    cx.update(|cx| {
        update_user_settings::<SessionSettings>(cx, None, move |c| c.reduce_motion = Some(value))
            .detach();
    });
    cx.run_until_parked();
}

fn effective(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| {
        let flag = oxikube_ui::motion::reduce_motion(cx);
        assert_eq!(flag, cx.reduce_motion(), "the ui helper reads GPUI's flag");
        flag
    })
}

#[gpui::test]
fn system_follows_the_os_preference(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    assert!(!effective(cx), "motion is on by default");
    cx.update(|cx| set_os_reduce_motion(cx, true));
    assert!(os_reduce_motion_of(cx));
    assert!(effective(cx));
    cx.update(|cx| set_os_reduce_motion(cx, false));
    assert!(!effective(cx));
}

fn os_reduce_motion_of(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| os_reduce_motion(cx))
}

#[gpui::test]
fn the_setting_overrides_the_os_in_both_directions(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    cx.update(|cx| set_os_reduce_motion(cx, true));
    assert!(effective(cx));
    set_setting(cx, ReduceMotionSetting::Off);
    assert!(
        !effective(cx),
        "off beats an OS that asks for reduced motion"
    );
    cx.update(|cx| set_os_reduce_motion(cx, false));
    set_setting(cx, ReduceMotionSetting::On);
    assert!(effective(cx), "on beats an OS that does not ask");
    set_setting(cx, ReduceMotionSetting::System);
    assert!(!effective(cx), "back to following the OS");
}

#[gpui::test]
fn the_os_preference_applies_without_a_settings_store(cx: &mut TestAppContext) {
    setup_without_settings(cx);
    cx.update(|cx| set_os_reduce_motion(cx, true));
    assert!(effective(cx));
}
