//! [`Gauge`]: a thin bar filled to a fraction, coloured by a [`Level`].
//!
//! Plain `div`s, no gpui-component. The fill is a fraction of the track's width, so a gauge
//! costs two quads and never re-measures. The text a screen reader gets is the gauge's `label`.

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px, relative,
};
use oxikube_ui::{ActiveTokens as _, u};

use super::face::Level;

/// Design pixels of the bar's height.
const HEIGHT: f32 = 6.0;

/// A bar filled to `fraction` of its width.
#[derive(IntoElement)]
pub struct Gauge {
    id: ElementId,
    fraction: Option<f32>,
    level: Level,
    label: SharedString,
}

impl Gauge {
    /// A gauge filled to `fraction` (`0.0..=1.0`; `None` draws the empty track and keeps the
    /// space, so a tile without a gauge does not change height).
    pub fn new(
        id: impl Into<ElementId>,
        fraction: Option<f32>,
        level: Level,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            fraction: fraction.map(|f| f.clamp(0.0, 1.0)),
            level,
            label: label.into(),
        }
    }
}

impl RenderOnce for Gauge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let fill = match self.level {
            Level::Good => colors.success,
            Level::Warn => colors.warning,
            Level::Critical => colors.error,
            Level::Neutral => colors.accent,
            Level::Muted => colors.text_muted,
        };
        let track = div()
            .id(self.id)
            .role(Role::ProgressIndicator)
            .aria_label(self.label)
            .w_full()
            .h(u(px(HEIGHT)))
            .rounded(u(px(HEIGHT / 2.0)))
            .overflow_hidden()
            .bg(colors.border_variant);
        match self.fraction {
            Some(fraction) => track.child(
                div()
                    .h_full()
                    .w(relative(fraction))
                    .rounded(u(px(HEIGHT / 2.0)))
                    .bg(fill),
            ),
            None => track,
        }
    }
}
