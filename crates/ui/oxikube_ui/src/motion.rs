//! Reduce-motion for animations (docs/PERFORMANCE.md rule 8: animations are short and off under
//! reduce-motion).
//!
//! The effective preference lives in GPUI's `App::reduce_motion`, which GPUI's own animations and
//! gpui-component already honour. The workspace (E05-S12) resolves it from the OS preference and
//! the `reduce_motion` setting and writes it there, so a view animating something checks
//! [`reduce_motion`] (or scales a time with [`duration`]) and nothing else.

use gpui::App;
use std::time::Duration;

/// Whether decorative motion is off: skip the animation and show the end state.
pub fn reduce_motion(cx: &App) -> bool {
    cx.reduce_motion()
}

/// `normal`, or [`Duration::ZERO`] under reduce-motion, for a transition that has no static
/// equivalent but may be instant.
pub fn duration(cx: &App, normal: Duration) -> Duration {
    if reduce_motion(cx) {
        Duration::ZERO
    } else {
        normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn follows_the_app_flag(cx: &mut TestAppContext) {
        let normal = Duration::from_millis(120);
        cx.update(|cx| {
            assert!(!reduce_motion(cx));
            assert_eq!(duration(cx, normal), normal);
            cx.set_reduce_motion(true);
            assert!(reduce_motion(cx));
            assert_eq!(duration(cx, normal), Duration::ZERO);
        });
    }
}
