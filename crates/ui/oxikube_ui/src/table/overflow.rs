//! The horizontal-overflow cue: a fade at the edge of the table where columns continue past the
//! view.
//!
//! The library draws a horizontal scrollbar, but a platform scrollbar hides until it is used, so a
//! table whose last columns are off screen gives no hint that they exist. The cue is drawn over the
//! table's edges, on the side that has more columns to scroll to, and never takes the mouse (it is
//! painted, not an element that hit-tests).
//!
//! It reads the library's scroll handle as it paints, so it follows the scroll and the first
//! layout in the same frame.

use gpui::{
    Bounds, Hsla, IntoElement, Pixels, ScrollHandle, Styled as _, canvas, fill, linear_color_stop,
    linear_gradient, point, px, size,
};

/// How wide the fade is, in design-time pixels (scaled by the UI zoom when drawn).
const FADE_WIDTH: f32 = 28.;

/// Where a table has more columns than the view shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HorizontalOverflow {
    /// Columns are scrolled off the left edge.
    pub left: bool,
    /// Columns continue past the right edge.
    pub right: bool,
}

impl HorizontalOverflow {
    /// The overflow of content scrolled by `offset_x` (zero or negative, as GPUI scroll offsets
    /// are) in a view that can scroll `max_x` (the hidden width, zero or more). Less than a pixel
    /// of overflow is rounding, not columns.
    pub fn from_scroll(offset_x: Pixels, max_x: Pixels) -> Self {
        let scrolled = -offset_x.min(Pixels::ZERO);
        let max_x = max_x.max(Pixels::ZERO);
        Self {
            left: max_x > px(1.) && scrolled > px(1.),
            right: max_x > px(1.) && max_x - scrolled > px(1.),
        }
    }

    /// Reads it from a table's horizontal scroll handle.
    pub fn of(handle: &ScrollHandle) -> Self {
        Self::from_scroll(handle.offset().x, handle.max_offset().x)
    }

    /// Whether any column is hidden by the view.
    pub fn any(self) -> bool {
        self.left || self.right
    }
}

/// The fade over the table's edges, in `color` (the table's background). Fills its parent.
pub(super) fn cue(handle: ScrollHandle, color: Hsla) -> impl IntoElement {
    let fade = crate::size::u(px(FADE_WIDTH));
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let overflow = HorizontalOverflow::of(&handle);
            let width = fade.min(bounds.size.width / 2.);
            let edge = |origin_x: Pixels| Bounds {
                origin: point(origin_x, bounds.origin.y),
                size: size(width, bounds.size.height),
            };
            let clear = color.opacity(0.);
            if overflow.right {
                let gradient = linear_gradient(
                    90.,
                    linear_color_stop(clear, 0.),
                    linear_color_stop(color, 1.),
                );
                window.paint_quad(fill(edge(bounds.right() - width), gradient));
            }
            if overflow.left {
                let gradient = linear_gradient(
                    270.,
                    linear_color_stop(clear, 0.),
                    linear_color_stop(color, 1.),
                );
                window.paint_quad(fill(edge(bounds.left()), gradient));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_follows_the_scroll_position() {
        let at = |offset: f32, max: f32| HorizontalOverflow::from_scroll(px(offset), px(max));
        assert_eq!(at(0., 0.), HorizontalOverflow::default(), "fits");
        assert!(!at(0., 0.5).any(), "a sub-pixel overflow is rounding");
        assert_eq!(
            at(0., 120.),
            HorizontalOverflow {
                left: false,
                right: true
            },
            "columns past the right edge"
        );
        assert_eq!(
            at(-60., 120.),
            HorizontalOverflow {
                left: true,
                right: true
            }
        );
        assert_eq!(
            at(-120., 120.),
            HorizontalOverflow {
                left: true,
                right: false
            },
            "scrolled to the end"
        );
    }
}
