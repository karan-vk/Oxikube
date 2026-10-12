//! `#[gpui::test]`s of the bar: segment geometry, clamping, and the absence of a usage segment.

use gpui::{
    Context, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px,
};

use super::AllocationBar;
use crate::chart::Tone;
use crate::tokens::ActiveTokens as _;

struct Host(Box<dyn Fn() -> AllocationBar>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.colors().background)
            .child(div().w(px(400.)).child((self.0)()))
    }
}

fn mount(
    cx: &mut TestAppContext,
    bar: impl Fn() -> AllocationBar + 'static,
) -> &mut VisualTestContext {
    cx.update(crate::init);
    let (_, vcx) = cx.add_window_view(move |_, _| Host(Box::new(bar)));
    vcx.run_until_parked();
    vcx
}

fn share(vcx: &mut VisualTestContext, part: &'static str, track: &'static str) -> Option<f32> {
    let part = vcx.debug_bounds(part)?;
    let track = vcx.debug_bounds(track)?;
    Some(f32::from(part.size.width) / f32::from(track.size.width))
}

#[gpui::test]
fn the_segments_sit_at_their_fractions(cx: &mut TestAppContext) {
    let vcx = mount(cx, || {
        AllocationBar::new(0.25)
            .limits_mark(Some(0.75))
            .usage(Some(0.5))
            .selector("b")
    });
    assert!((share(vcx, "b-fill", "b-bar").unwrap() - 0.25).abs() < 0.01);
    assert!((share(vcx, "b-usage", "b-bar").unwrap() - 0.5).abs() < 0.01);
    let track = vcx.debug_bounds("b-bar").unwrap();
    let tick = vcx.debug_bounds("b-limits").unwrap();
    let at = f32::from(tick.origin.x - track.origin.x) / f32::from(track.size.width);
    assert!(
        (at - 0.74).abs() < 0.02,
        "the tick starts just before 0.75: {at}"
    );
    assert!(
        tick.size.height >= track.size.height,
        "the tick spans the track"
    );
    let usage = vcx.debug_bounds("b-usage").unwrap();
    assert!(
        usage.size.height < track.size.height,
        "the usage bar is thinner"
    );
    assert_eq!(
        usage.bottom(),
        track.bottom(),
        "and sits on the bottom edge"
    );
}

#[gpui::test]
fn no_usage_and_no_limits_draw_no_segments(cx: &mut TestAppContext) {
    let vcx = mount(cx, || AllocationBar::new(0.5).selector("b"));
    assert!(vcx.debug_bounds("b-fill").is_some());
    assert!(
        vcx.debug_bounds("b-usage").is_none(),
        "no reading is not a zero reading"
    );
    assert!(vcx.debug_bounds("b-limits").is_none());
}

#[gpui::test]
fn fractions_are_clamped_to_the_track(cx: &mut TestAppContext) {
    let vcx = mount(cx, || {
        AllocationBar::new(7.0)
            .usage(Some(-3.0))
            .limits_mark(Some(f32::NAN))
            .tone(Tone::Critical)
            .selector("b")
    });
    assert!((share(vcx, "b-fill", "b-bar").unwrap() - 1.0).abs() < 0.01);
    assert!(share(vcx, "b-usage", "b-bar").unwrap() < 0.01);
    let track = vcx.debug_bounds("b-bar").unwrap();
    let tick = vcx.debug_bounds("b-limits").unwrap();
    assert!(
        tick.origin.x <= track.origin.x + px(1.),
        "NaN reads as the start"
    );
}
