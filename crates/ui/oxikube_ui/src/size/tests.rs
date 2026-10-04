use super::*;
use gpui::TestAppContext;

#[test]
fn scale_is_clamped_and_rejects_nan() {
    assert_eq!(UiScale::new(0.1).factor(), UiScale::MIN);
    assert_eq!(UiScale::new(9.0).factor(), UiScale::MAX);
    assert_eq!(UiScale::new(f32::NAN), UiScale::IDENTITY);
    assert_eq!(UiScale::new(1.25).factor(), 1.25);
}

#[test]
fn unscaled_round_trips_through_any_zoom() {
    for scale in [0.5, 0.8, 1.0, 1.25, 1.5, 2.0, 3.0] {
        let scale = UiScale::new(scale);
        for stored in [120.0_f32, 240.0, 333.0] {
            let on_screen = Unscaled(stored).at(scale);
            let back = Unscaled::from_scaled(on_screen, scale);
            assert!(
                (back.0 - stored).abs() < 1e-3,
                "{stored} at {scale:?} came back as {}",
                back.0
            );
        }
    }
}

#[test]
fn unscaled_serialises_as_a_bare_number() {
    let json = serde_json::to_string(&Unscaled(240.0)).unwrap();
    assert_eq!(json, "240.0");
    let back: Unscaled = serde_json::from_str(&json).unwrap();
    assert_eq!(back, Unscaled(240.0));
}

#[gpui::test]
fn u_follows_the_ui_zoom(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
        assert_eq!(u(px(10.)), px(10.));
        assert_eq!(Unscaled(200.0).to_pixels(), px(200.));

        set_ui_scale(cx, UiScale::new(1.5));
        assert_eq!(UiScale::get(cx).factor(), 1.5);
        assert_eq!(u(px(10.)), px(15.));
        assert_eq!(Unscaled(200.0).to_pixels(), px(300.));

        set_ui_scale(cx, UiScale::IDENTITY);
        assert_eq!(u(px(10.)), px(10.));
    });
}
