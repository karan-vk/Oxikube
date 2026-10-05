//! Motion policy for the overlay layers: animations are short and can be switched off.
//!
//! docs/PERFORMANCE.md rule 8: animations are at most [`MAX_ANIMATION`] long, cheap, and off
//! under reduce-motion. The layers ask [`animation_duration`] instead of picking a duration, so
//! one switch ([`set_reduce_motion`]) turns every animation here into an instant state change.
//!
//! GPUI has no portable "reduce motion" query, so the flag is a plain global that the binary sets
//! from the user's setting (the settings entry lands with the settings UI, E21).

use std::time::Duration;

use gpui::{App, Global};

/// The longest animation any overlay plays.
pub const MAX_ANIMATION: Duration = Duration::from_millis(150);

struct ReduceMotion(bool);

impl Global for ReduceMotion {}

/// Turns reduce-motion on or off for the whole app.
pub fn set_reduce_motion(cx: &mut App, reduce: bool) {
    cx.set_global(ReduceMotion(reduce));
}

/// Whether animations are switched off. Off (animations allowed) until [`set_reduce_motion`].
pub fn reduce_motion(cx: &App) -> bool {
    cx.try_global::<ReduceMotion>().is_some_and(|flag| flag.0)
}

/// How long an animation that would like `wanted` runs: `wanted` clamped to [`MAX_ANIMATION`],
/// or `None` (do not animate) under reduce-motion.
pub fn animation_duration(cx: &App, wanted: Duration) -> Option<Duration> {
    (!reduce_motion(cx)).then(|| wanted.min(MAX_ANIMATION))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn durations_are_clamped_and_switch_off(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert!(!reduce_motion(cx));
            assert_eq!(
                animation_duration(cx, Duration::from_millis(100)),
                Some(Duration::from_millis(100))
            );
            assert_eq!(
                animation_duration(cx, Duration::from_millis(400)),
                Some(MAX_ANIMATION)
            );
            set_reduce_motion(cx, true);
            assert_eq!(animation_duration(cx, Duration::from_millis(100)), None);
            set_reduce_motion(cx, false);
            assert!(animation_duration(cx, Duration::from_millis(100)).is_some());
        });
    }
}
