//! The effective reduce-motion flag.
//!
//! GPUI does not read the OS preference itself (`App::reduce_motion` starts `false`), so the
//! platform glue feeds it in with [`set_os_reduce_motion`]; the `reduce_motion` setting then
//! decides ([`resolve_reduce_motion`]) and the result is written to GPUI's flag, which GPUI's
//! animations and gpui-component already check. Views use `oxikube_ui::motion::reduce_motion`.

use gpui::{App, Global};
use oxikube_settings::Settings as _;

use super::settings::{ReduceMotionSetting, SessionSettings};

/// The OS preference as the platform glue last reported it.
#[derive(Default)]
struct OsReduceMotion(bool);

impl Global for OsReduceMotion {}

/// The effective flag: an `on` or `off` setting beats the OS value, `system` follows it.
pub fn resolve_reduce_motion(setting: ReduceMotionSetting, os: bool) -> bool {
    match setting {
        ReduceMotionSetting::On => true,
        ReduceMotionSetting::Off => false,
        ReduceMotionSetting::System => os,
    }
}

/// The OS reduce-motion preference last reported by [`set_os_reduce_motion`] (`false` until then).
pub fn os_reduce_motion(cx: &App) -> bool {
    cx.try_global::<OsReduceMotion>().is_some_and(|os| os.0)
}

/// Reports the OS reduce-motion preference (at start-up and whenever the platform notices a
/// change) and re-applies the effective flag.
pub fn set_os_reduce_motion(cx: &mut App, reduce: bool) {
    cx.set_global(OsReduceMotion(reduce));
    apply(cx);
}

/// Writes the effective flag to GPUI. Skips the write when nothing changed, so the windows are
/// only refreshed on a real change.
pub(super) fn apply(cx: &mut App) {
    let setting = SessionSettings::try_get(cx)
        .map(|settings| settings.reduce_motion)
        .unwrap_or_default();
    cx.set_reduce_motion(resolve_reduce_motion(setting, os_reduce_motion(cx)));
}
