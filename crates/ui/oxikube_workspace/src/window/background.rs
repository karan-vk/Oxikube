//! A window in the background keeps nothing focused (E07-F512).
//!
//! A focused text field blinks its caret: the library notifies it every 500 ms and the window
//! redraws for each blink, forever, whether or not anyone is looking. In the foreground that is
//! the caret; in a window that lost the key (another app in front, the window still visible
//! beside it) it is two frames a second of nothing, which is most of the app's idle CPU. Zed
//! stops blinking when its window is inactive for the same reason.
//!
//! The field stops blinking when it loses the focus, so [`follow`] hands the focus back to
//! nobody while the window is inactive and gives it back to the same element when the window
//! becomes active again, unless something else took the focus meanwhile. Nothing visible changes:
//! an inactive window draws no caret anyway.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Context, Subscription, WeakFocusHandle, Window};

/// Parks the window's focus while it is inactive, for as long as the returned subscription
/// lives. Install it once per window, from the view that owns the window's content.
pub(super) fn follow<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Subscription {
    let parked: Rc<RefCell<Option<WeakFocusHandle>>> = Rc::default();
    cx.observe_window_activation(window, move |_, window, cx| {
        if window.is_window_active() {
            let Some(focus) = parked.borrow_mut().take().and_then(|weak| weak.upgrade()) else {
                return;
            };
            if window.focused(cx).is_none() {
                window.focus(&focus, cx);
            }
        } else if let Some(focus) = window.focused(cx) {
            *parked.borrow_mut() = Some(focus.downgrade());
            window.blur(cx);
        }
    })
}
