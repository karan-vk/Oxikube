//! Glyph warm-up (E05-P602): a theme switch draws its first frame without rasterising text.
//!
//! GPUI's glyph atlas keys a glyph by its *dilation* too: on macOS the stroke thickening
//! CoreGraphics applies depends on the luminance of the text colour (five levels), so light text
//! on a dark theme and dark text on a light theme are different glyphs. Without warm-up, the first
//! frame after switching to a theme whose text sits at other levels misses the atlas for every
//! glyph on screen and rasterises each one with CoreText, one by one, in the frame.
//!
//! The warm-up moves that work out of the frame without changing what is drawn:
//!
//! 1. [`GlyphWarmPlatform`] hands GPUI a decorated text system ([`GlyphWarmer::text_system`]):
//!    the platform's, which also records every glyph GPUI rasterises (the glyphs on screen, in
//!    the atlas) and the level it drew it at.
//! 2. [`install`] keeps a [`DilationPlan`] current: each colour slot of the active theme paired
//!    with the same slot of every installed theme, mapped to levels by the platform's own
//!    `glyph_dilation_for_color`. It is recomputed when a theme is selected or the registry
//!    changes (a hot-reloaded `themes/` directory, a `settings.json` edit).
//! 3. A worker thread rasterises every drawn glyph at the plan's other levels with the same
//!    platform calls GPUI would make, between frames (it waits for the text system to be idle
//!    for [`QUIET`]), and keeps the bitmaps (at most [`PREPARED_BUDGET`] bytes).
//! 4. When a switch draws, GPUI misses the atlas as before and asks the text system for the
//!    glyph's bounds and bitmap: the prepared ones answer, so the frame only uploads them.
//!
//! On platforms whose text system draws every colour at one level (Linux, Windows) the plan is
//! empty and nothing runs.
//!
//! | File | Holds |
//! |---|---|
//! | `plan` | [`DilationPlan`] |
//! | `warmer` | [`GlyphWarmer`]: the glyph records, the queue, the prepared bitmaps, the worker |
//! | `text_system` | the decorated `PlatformTextSystem` |
//! | `platform` | [`GlyphWarmPlatform`] |

mod plan;
mod platform;
mod text_system;
mod warmer;

#[cfg(test)]
mod tests;

pub use plan::{DilationPlan, LEVELS};
pub use platform::GlyphWarmPlatform;
pub use warmer::{GlyphWarmer, PREPARED_BUDGET, QUIET, WarmStats};

use crate::global::ActiveTheme;
use crate::registry::ThemeRegistry;
use gpui::{App, Global, Platform};
use std::rc::Rc;

/// The app's [`GlyphWarmer`], a global once [`install`]ed.
#[derive(Clone)]
pub struct GlyphWarm(pub GlyphWarmer);

impl Global for GlyphWarm {}

/// `inner` with a warmer around its text system: the platform to build the `Application` on,
/// and the warmer to [`install`] once the app runs.
pub fn wrap_platform(inner: Rc<dyn Platform>) -> (Rc<dyn Platform>, GlyphWarmer) {
    let warmer = GlyphWarmer::new(inner.text_system());
    (Rc::new(GlyphWarmPlatform::new(inner, &warmer)), warmer)
}

/// Makes `warmer` the app's and keeps its plan current with the active theme and the registry.
/// Call before [`crate::init`] (the plan is made when both globals are set) or after it.
pub fn install(warmer: GlyphWarmer, cx: &mut App) {
    cx.set_global(GlyphWarm(warmer));
    cx.observe_global::<ActiveTheme>(replan).detach();
    cx.observe_global::<ThemeRegistry>(replan).detach();
    replan(cx);
}

/// Recomputes the plan from the active theme to every installed one.
fn replan(cx: &mut App) {
    let (Some(warm), Some(registry), Some(active)) = (
        cx.try_global::<GlyphWarm>(),
        cx.try_global::<ThemeRegistry>(),
        cx.try_global::<ActiveTheme>(),
    ) else {
        return;
    };
    let warmer = &warm.0;
    let themes: Vec<_> = registry
        .names()
        .iter()
        .filter_map(|name| registry.get(name))
        .collect();
    let plan = DilationPlan::for_switch(&active.0, themes.iter().map(|t| &**t), |color| {
        warmer.dilation_for_color(color)
    });
    warmer.set_plan(plan);
}
