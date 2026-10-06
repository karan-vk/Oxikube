//! Focus trapping for the modal layer: Tab and Shift-Tab cycle inside the modal.
//!
//! GPUI's tab order is window-wide, so "the next tab stop" can sit behind the scrim, and its
//! backwards step does not wrap reliably at the end of the order in this snapshot. The trap
//! therefore only ever steps *forward*: it walks one full lap of the window's tab stops from the
//! current focus, keeps those that are inside the modal (in lap order), and picks the first of
//! them for Tab or the last for Shift-Tab. That makes the modal's own stops a closed cycle in
//! both directions. A lap is bounded, so a window without tab stops cannot spin.

use gpui::{App, FocusHandle, Window};

/// More steps than any window has tab stops.
const MAX_STEPS: usize = 512;

/// Moves focus to the next (`forward`) or previous tab stop *inside* `container`, wrapping at the
/// ends. Returns whether focus is now inside it; false means the modal has no tab stop (and focus
/// is back where it started).
pub(super) fn cycle(
    container: &FocusHandle,
    forward: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let start = window.focused(cx);
    let mut visited: Vec<FocusHandle> = start.iter().cloned().collect();
    let mut inside: Vec<FocusHandle> = Vec::new();
    for _ in 0..MAX_STEPS {
        window.focus_next(cx);
        let Some(now) = window.focused(cx) else {
            break;
        };
        if visited.contains(&now) {
            break;
        }
        if container.contains(&now, window) {
            inside.push(now.clone());
        }
        visited.push(now);
    }
    // Back at the start only when the whole lap came back to it; otherwise the lap saw every stop
    // once and `inside` is complete too.
    let target = if forward {
        inside.first()
    } else {
        inside.last()
    }
    .cloned()
    .or_else(|| {
        start
            .clone()
            .filter(|start| container.contains(start, window))
    });
    match target {
        Some(target) => {
            target.focus(window, cx);
            true
        }
        None => {
            if let Some(start) = start {
                start.focus(window, cx);
            }
            false
        }
    }
}
