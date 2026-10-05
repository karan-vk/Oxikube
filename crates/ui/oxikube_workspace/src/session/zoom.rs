//! UI zoom: `view::ZoomIn`, `view::ZoomOut`, `view::ZoomReset`.
//!
//! The zoom factor is [`oxikube_ui::UiScale`]. Setting it re-themes the component library with
//! scaled font size and radii, and the `Root` turns the scaled font size into the window's rem
//! size at the start of every frame, so the next frame lays out at the new zoom: sizes written
//! through `oxikube_ui::u` and everything in rems follow, with no stale value left over from the
//! old zoom. The change is persisted as `ui_scale` in `settings.json`.

use gpui::{App, Global, UpdateGlobal as _, actions};
use oxikube_settings::{Settings as _, update_user_settings};
use oxikube_ui::{UiScale, set_ui_scale};

use super::settings::SessionSettings;

actions!(
    view,
    [
        /// Make the UI one zoom step larger.
        ZoomIn,
        /// Make the UI one zoom step smaller.
        ZoomOut,
        /// Set the UI zoom back to 100 %.
        ZoomReset,
    ]
);

/// The zoom levels the keys step through (Chrome's ladder, widened to the supported range).
pub const ZOOM_STEPS: [f32; 15] = [
    0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// Tolerance when comparing zoom factors (they are `f32`s written to a JSON file).
const EPSILON: f32 = 1e-3;

/// The next zoom level above `current`, or the largest when there is none.
pub fn zoom_in_from(current: UiScale) -> UiScale {
    let factor = current.factor();
    let next = ZOOM_STEPS
        .iter()
        .copied()
        .find(|step| *step > factor + EPSILON)
        .unwrap_or(UiScale::MAX);
    UiScale::new(next)
}

/// The next zoom level below `current`, or the smallest when there is none.
pub fn zoom_out_from(current: UiScale) -> UiScale {
    let factor = current.factor();
    let next = ZOOM_STEPS
        .iter()
        .copied()
        .rev()
        .find(|step| *step < factor - EPSILON)
        .unwrap_or(UiScale::MIN);
    UiScale::new(next)
}

/// Zooms in one step.
pub fn zoom_in(cx: &mut App) {
    set_zoom(cx, zoom_in_from(UiScale::get(cx)));
}

/// Zooms out one step.
pub fn zoom_out(cx: &mut App) {
    set_zoom(cx, zoom_out_from(UiScale::get(cx)));
}

/// Sets the zoom back to 100 %.
pub fn zoom_reset(cx: &mut App) {
    set_zoom(cx, UiScale::IDENTITY);
}

/// Applies `scale` to the UI now and persists it.
pub fn set_zoom(cx: &mut App, scale: UiScale) {
    if (UiScale::get(cx).factor() - scale.factor()).abs() < EPSILON {
        return;
    }
    set_ui_scale(cx, scale);
    persist(cx, scale);
}

/// Writes `ui_scale` to the user's settings file, when there is a settings store.
fn persist(cx: &mut App, scale: UiScale) {
    if SessionSettings::try_get(cx).is_none() {
        return;
    }
    cx.default_global::<ZoomWrites>().0 += 1;
    let factor = scale.factor();
    let write = update_user_settings::<SessionSettings>(cx, None, move |content| {
        content.ui_scale = Some(factor);
    });
    cx.spawn(async move |cx| {
        let result = write.await;
        cx.update(|cx| finish_write(cx, result.err()));
    })
    .detach();
}

/// Counts the zoom writes in flight, so a settings change they cause is not applied back over a
/// newer zoom.
#[derive(Default)]
struct ZoomWrites(usize);

impl Global for ZoomWrites {}

/// Whether a zoom is still being written to `settings.json`.
pub(super) fn write_pending(cx: &App) -> bool {
    cx.try_global::<ZoomWrites>()
        .is_some_and(|writes| writes.0 > 0)
}

/// A write ended. A failed write keeps the zoom on screen (the user asked for it) and logs; once
/// the last one ends the settings are re-applied, which picks up an edit of the file made by hand
/// in the meantime.
fn finish_write(cx: &mut App, error: Option<oxikube_domain::OxiError>) {
    ZoomWrites::update_global(cx, |writes, _| writes.0 = writes.0.saturating_sub(1));
    if let Some(error) = error {
        tracing::warn!(%error, "could not save the UI zoom to settings.json");
    } else if !write_pending(cx) {
        super::settings::apply(cx);
    }
}

/// Registers the zoom action handlers (the key bindings are in the keymap files).
pub(super) fn register(cx: &mut App) {
    cx.on_action(|_: &ZoomIn, cx| zoom_in(cx));
    cx.on_action(|_: &ZoomOut, cx| zoom_out(cx));
    cx.on_action(|_: &ZoomReset, cx| zoom_reset(cx));
}
