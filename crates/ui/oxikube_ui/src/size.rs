//! Zoom-safe sizes.
//!
//! UI zoom multiplies every pixel size by one factor. Resizable panels and docks work in absolute
//! pixels (a Kubyl lesson), so nothing scales for free: every literal size in a view goes through
//! [`u`], and every size that is *stored* (dock widths, panel heights) is kept in its unscaled
//! form ([`Unscaled`]) and converted on read.
//!
//! The factor lives in a [`UiScale`] global (the source of truth, set with [`set_ui_scale`]) and in
//! a thread-local mirror so [`u`] needs no context. GPUI's foreground state is single-threaded and
//! every `#[gpui::test]` runs on its own thread, so the mirror cannot leak between tests.

use crate::theme_bridge;
use gpui::{App, Global, Pixels, px};

pub use gpui_component::{Sizable, Size as ControlSize};
use serde::{Deserialize, Serialize};
use std::cell::Cell;

thread_local! {
    static SCALE: Cell<f32> = const { Cell::new(1.0) };
}

/// The UI zoom factor (`1.0` = 100 %). Always inside [`UiScale::MIN`]..=[`UiScale::MAX`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiScale(f32);

impl Global for UiScale {}

impl UiScale {
    /// Smallest zoom (50 %).
    pub const MIN: f32 = 0.5;
    /// Largest zoom (300 %).
    pub const MAX: f32 = 3.0;
    /// No zoom.
    pub const IDENTITY: UiScale = UiScale(1.0);

    /// A scale clamped into the supported range. Non-finite input yields `1.0`.
    pub fn new(factor: f32) -> Self {
        if factor.is_finite() {
            UiScale(factor.clamp(Self::MIN, Self::MAX))
        } else {
            Self::IDENTITY
        }
    }

    /// The factor as a plain number.
    pub fn factor(self) -> f32 {
        self.0
    }

    /// The scale stored in `cx`, or identity before [`crate::init`].
    pub fn get(cx: &App) -> UiScale {
        cx.try_global::<UiScale>().copied().unwrap_or_default()
    }
}

impl Default for UiScale {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The current zoom factor as seen by [`u`] on this thread.
pub fn current_scale() -> f32 {
    SCALE.with(Cell::get)
}

/// Scales a design-time pixel size by the current UI zoom.
///
/// ```ignore
/// div().p(u(px(8.))).h(u(px(28.)))
/// ```
pub fn u(size: Pixels) -> Pixels {
    size * current_scale()
}

/// Sets the UI zoom: updates the global, the thread-local mirror behind [`u`], and re-applies the
/// active tokens so the component library's font size and radii follow. Refreshes all windows.
pub fn set_ui_scale(cx: &mut App, scale: UiScale) {
    SCALE.with(|s| s.set(scale.factor()));
    cx.set_global(scale);
    theme_bridge::reapply(cx);
}

/// Resets the zoom to 100 % without touching the theme (used by [`crate::init`], which applies the
/// tokens itself right afterwards).
pub(crate) fn reset(cx: &mut App) {
    SCALE.with(|s| s.set(1.0));
    cx.set_global(UiScale::IDENTITY);
}

/// A size as stored on disk: **unscaled** logical pixels.
///
/// Dock and panel sizes are persisted in this form so a layout saved at 150 % zoom restores
/// correctly at 100 %. Convert with [`Unscaled::to_pixels`] when reading and with
/// [`Unscaled::from_scaled`] when the user drags a resize handle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Unscaled(pub f32);

impl Unscaled {
    /// Unscales a pixel size measured on screen at zoom `scale`.
    pub fn from_scaled(scaled: Pixels, scale: UiScale) -> Self {
        Unscaled(f32::from(scaled) / scale.factor())
    }

    /// The on-screen size at the current zoom (the same factor [`u`] applies).
    pub fn to_pixels(self) -> Pixels {
        u(px(self.0))
    }

    /// The on-screen size at an explicit zoom.
    pub fn at(self, scale: UiScale) -> Pixels {
        px(self.0 * scale.factor())
    }
}

#[cfg(test)]
mod tests;
