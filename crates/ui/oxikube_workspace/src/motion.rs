//! Motion policy for the overlay layers: animations are short and can be switched off.
//!
//! docs/PERFORMANCE.md rule 8: animations are at most [`MAX_ANIMATION`] long, cheap, and off
//! under reduce-motion. The layers ask [`animation_duration`] instead of picking a duration, so
//! the one app-wide switch (GPUI's `App::reduce_motion`, resolved from the OS preference and the
//! `reduce_motion` setting by [`crate::session`] and read through `oxikube_ui::motion`) turns
//! every animation here into an instant state change.

use std::time::Duration;

use gpui::App;

/// The longest animation any overlay plays.
pub const MAX_ANIMATION: Duration = Duration::from_millis(150);

/// How long an animation that would like `wanted` runs: `wanted` clamped to [`MAX_ANIMATION`],
/// or `None` (do not animate) under reduce-motion.
pub fn animation_duration(cx: &App, wanted: Duration) -> Option<Duration> {
    (!oxikube_ui::motion::reduce_motion(cx)).then(|| wanted.min(MAX_ANIMATION))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn durations_are_clamped_and_switch_off(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(
                animation_duration(cx, Duration::from_millis(100)),
                Some(Duration::from_millis(100))
            );
            assert_eq!(
                animation_duration(cx, Duration::from_millis(400)),
                Some(MAX_ANIMATION)
            );
            cx.set_reduce_motion(true);
            assert_eq!(animation_duration(cx, Duration::from_millis(100)), None);
            cx.set_reduce_motion(false);
            assert!(animation_duration(cx, Duration::from_millis(100)).is_some());
        });
    }
}
