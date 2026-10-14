//! [`ObjectEvents`]: the events index as the views reach it.

use gpui::{App, Global};
use oxikube_app::WarningBadges;

/// The app's [`WarningBadges`] for the views: the tables' badge column and the detail drawer's
/// Events tab read it. Installed once by the binary when the first window mounts; a view built
/// before that has no badge column and an Events tab that says events are unavailable.
pub struct ObjectEvents {
    badges: WarningBadges,
}

impl Global for ObjectEvents {}

impl ObjectEvents {
    /// Makes `badges` what every table and drawer reads. A second call replaces the first.
    pub fn install(badges: WarningBadges, cx: &mut App) {
        cx.set_global(Self { badges });
    }

    /// The installed badges.
    pub fn get(cx: &App) -> Option<WarningBadges> {
        cx.try_global::<Self>().map(|events| events.badges.clone())
    }
}
