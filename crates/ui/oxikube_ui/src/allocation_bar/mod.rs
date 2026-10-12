//! [`AllocationBar`]: how full something is, in three segments.
//!
//! One look for "requests, limits and live usage against a capacity" (a node's allocatable, a
//! namespace's quota): a track, the **fill** (what is requested or used up) in a status colour,
//! a thin **usage** bar along the bottom edge, and a **limits** tick where the limits end. It is
//! a stateless element built per frame from three fractions; the caller prints the numbers beside
//! it, because colour alone never carries the meaning (the bar is `0.0..=1.0` of the track, the
//! text says the real figure, which can be past it).
//!
//! The status colours are the chart palette's, so they keep their contrast against the track in
//! every theme. Plain `div`s: nothing here depends on gpui-component.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Styled as _, Window,
    div, prelude::FluentBuilder as _, px, relative,
};

use crate::chart::{ChartPalette, Tone};
use crate::tokens::ActiveTokens as _;
use crate::u;

/// The track height, before the UI zoom.
const HEIGHT: f32 = 12.;
/// The usage bar's height, before the UI zoom.
const USAGE_HEIGHT: f32 = 4.;
/// The limits tick's width, before the UI zoom.
const TICK_WIDTH: f32 = 2.;

/// A fraction of the track, clamped to `0.0..=1.0` (NaN reads as empty).
fn clamp(fraction: f32) -> f32 {
    if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    }
}

/// A three-segment bar. See the [module docs](self).
#[derive(IntoElement)]
pub struct AllocationBar {
    fill: f32,
    tone: Tone,
    limits_mark: Option<f32>,
    usage: Option<f32>,
    selector: Option<String>,
}

impl AllocationBar {
    /// A bar filled to `fill` of its track (`0.0..=1.0`), in the `Good` colour.
    pub fn new(fill: f32) -> Self {
        Self {
            fill: clamp(fill),
            tone: Tone::Good,
            limits_mark: None,
            usage: None,
            selector: None,
        }
    }

    /// The fill's status colour.
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Draws the limits tick at `at` of the track.
    #[must_use]
    pub fn limits_mark(mut self, at: Option<f32>) -> Self {
        self.limits_mark = at.map(clamp);
        self
    }

    /// Draws the live-usage bar to `fraction` of the track; `None` draws none (no reading is not
    /// a reading of zero).
    #[must_use]
    pub fn usage(mut self, fraction: Option<f32>) -> Self {
        self.usage = fraction.map(clamp);
        self
    }

    /// Test selectors (`debug_selector`): `{prefix}-bar` for the track, `{prefix}-fill`,
    /// `{prefix}-usage` and `{prefix}-limits` for the segments.
    #[must_use]
    pub fn selector(mut self, prefix: impl Into<String>) -> Self {
        self.selector = Some(prefix.into());
        self
    }
}

impl RenderOnce for AllocationBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let palette = ChartPalette::from_tokens(&tokens);
        let name = |part: &'static str| {
            let prefix = self.selector.clone();
            move || format!("{}-{part}", prefix.unwrap_or_default())
        };
        let named = self.selector.is_some();
        let track = div()
            .when(named, |d| d.debug_selector(name("bar")))
            .relative()
            .h(u(px(HEIGHT)))
            .w_full()
            .rounded(u(tokens.radius.sm))
            .bg(tokens.colors.element)
            .overflow_hidden()
            .child(
                div()
                    .when(named, |d| d.debug_selector(name("fill")))
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(relative(self.fill))
                    .bg(palette.tone(self.tone)),
            );
        track
            .when_some(self.usage, |track, fraction| {
                track.child(
                    div()
                        .when(named, |d| d.debug_selector(name("usage")))
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .h(u(px(USAGE_HEIGHT)))
                        .w(relative(fraction))
                        .bg(palette.info),
                )
            })
            .when_some(self.limits_mark, |track, at| {
                track.child(
                    div()
                        .when(named, |d| d.debug_selector(name("limits")))
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(u(px(TICK_WIDTH)))
                        .left(relative((at - 0.01).max(0.0)))
                        .bg(tokens.colors.text),
                )
            })
    }
}

#[cfg(test)]
mod tests;
