//! Whole rows at the top (E08-U556). Following the log puts the newest row at the bottom edge,
//! so when the body is not a whole number of rows tall the oldest row on screen is cut in half
//! under the toolbar. The slack (the body's height modulo one row) is left empty above the rows
//! instead, so every row on screen is whole. A log that does not fill the body has no slack: its
//! first row is at the top. The wrapped list has rows of their own heights and
//! is left alone.
//!
//! The body's height is read by a `canvas` that does not take part in layout (absolutely
//! positioned over the body), so the rows' size cannot feed back into the measurement.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Context, Entity, IntoElement, Pixels, Styled as _, canvas, px};

use super::LogView;

/// Slack changes smaller than this (in pixels) are not worth a frame.
const EPSILON: f32 = 0.5;

/// The empty space above the unwrapped rows, as measured on the last frame.
#[derive(Clone, Default)]
pub(crate) struct RowSnap {
    /// The body's height modulo one row.
    slack: Rc<Cell<Pixels>>,
    /// The body's height.
    height: Rc<Cell<Pixels>>,
}

impl RowSnap {
    /// The slack to leave above `rows` rows of height `row`: the body's measured slack once the
    /// rows fill the body, none before.
    pub(crate) fn slack(&self, rows: usize, row: Pixels) -> Pixels {
        if row * rows as f32 > self.height.get() {
            self.slack.get()
        } else {
            px(0.)
        }
    }

    /// Takes the measurement of a body `height` tall with rows `row` tall; true when the slack
    /// moved (the view then draws once more).
    fn measure(&self, height: Pixels, row: Pixels) -> bool {
        let slack = slack_of(height, row);
        let moved = (slack - self.slack.get()).abs() > px(EPSILON)
            || (height - self.height.get()).abs() > px(EPSILON);
        self.slack.set(slack);
        self.height.set(height);
        moved
    }
}

/// `height` modulo `row` (zero for a degenerate row or height).
fn slack_of(height: Pixels, row: Pixels) -> Pixels {
    if row <= px(0.) || height <= row {
        return px(0.);
    }
    let rows = (height / row).floor();
    height - row * rows
}

impl LogView {
    /// The probe that measures the body for [`RowSnap`]; place it inside the body.
    pub(crate) fn body_probe(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let snap = self.snap.clone();
        let row = self.row_height();
        let view: Entity<LogView> = cx.entity();
        canvas(
            move |bounds, _, cx| {
                if snap.measure(bounds.size.height, row) {
                    view.update(cx, |_, cx| cx.notify());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slack_is_what_is_left_after_whole_rows() {
        assert_eq!(slack_of(px(105.), px(20.)), px(5.));
        assert_eq!(slack_of(px(100.), px(20.)), px(0.));
        assert_eq!(slack_of(px(15.), px(20.)), px(0.), "less than a row");
        assert_eq!(slack_of(px(100.), px(0.)), px(0.));
    }

    #[test]
    fn a_stable_height_does_not_ask_for_another_frame() {
        let snap = RowSnap::default();
        assert!(snap.measure(px(105.), px(20.)));
        assert_eq!(snap.slack(100, px(20.)), px(5.));
        assert_eq!(
            snap.slack(5, px(20.)),
            px(0.),
            "a short log is not pushed down"
        );
        assert!(!snap.measure(px(105.), px(20.)));
        assert!(snap.measure(px(110.), px(20.)));
        assert_eq!(snap.slack(100, px(20.)), px(10.));
    }
}
