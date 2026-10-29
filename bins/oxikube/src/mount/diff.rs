//! The resource diff (E20-S09): `resource::DiffWith` over the app's [`DiffService`].
//!
//! [`install`] makes the one service and keeps its ignore list in step with the `diff.ignore_paths`
//! setting: the list is read once at start-up and again on every change of the setting (the
//! settings store only wakes the observer when the value moved), so an edit applies to the next
//! diff with no restart.

use gpui::{App, Subscription};
use oxikube_app::ClusterSessionManager;
use oxikube_app::diff::{DiffService, IgnoreList, IgnorePaths};
use oxikube_settings::{DiffSettings, Settings as _};

/// The service, and the subscription that feeds it the setting (held for the window's life).
pub struct DiffWiring {
    /// The service `resource::DiffWith` runs on.
    pub service: DiffService,
    /// Keeps the ignore list following the setting.
    pub follow: Subscription,
}

/// Starts the service over `sessions` with the ignore list the setting holds now.
pub fn install(sessions: ClusterSessionManager, cx: &mut App) -> DiffWiring {
    let ignore = IgnoreList::new(paths(&DiffSettings::resolve(cx)));
    let follow = DiffSettings::observe(cx, {
        let ignore = ignore.clone();
        move |cx| ignore.set(paths(&DiffSettings::resolve(cx)))
    });
    DiffWiring {
        service: DiffService::new(sessions, ignore),
        follow,
    }
}

fn paths(settings: &DiffSettings) -> IgnorePaths {
    IgnorePaths::new(&settings.ignore_paths)
}

#[cfg(test)]
mod tests {
    use oxikube_app::diff::DEFAULT_IGNORE_PATHS;

    use super::*;

    #[test]
    fn the_apps_default_list_is_the_settings_default() {
        assert_eq!(
            DiffSettings::default_ignore_paths(),
            DEFAULT_IGNORE_PATHS.to_vec()
        );
    }
}
