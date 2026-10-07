//! Feature crates' `init(cx)`.
//!
//! Every feature crate exposes `pub fn init(cx: &mut App)` that registers its actions, settings,
//! item builders and globals (Zed's pattern). A feature epic adds its crate here, one line, in
//! dependency order; nothing else in `main` changes. Keep each `init` small and synchronous: it
//! runs before the first frame (`docs/PERFORMANCE.md`), so anything heavy (the extension host,
//! discovery, the agent client) registers its action and starts on first use instead.

use gpui::App;

use super::stage::{Stage, StartupReport};

/// One feature crate's init.
#[derive(Clone, Copy)]
pub struct Feature {
    /// Crate name, for the log.
    pub name: &'static str,
    /// Its `init(cx)`.
    pub init: fn(&mut App),
}

/// The feature crates, in the order their `init`s run.
pub const FEATURES: &[Feature] = &[
    Feature {
        name: "oxikube_catalog_ui",
        init: oxikube_catalog_ui::init,
    },
    Feature {
        name: "oxikube_resources_ui",
        init: oxikube_resources_ui::init,
    },
    Feature {
        name: "oxikube_terminal",
        init: oxikube_terminal::init,
    },
    Feature {
        name: "cluster_prefs",
        init: follow_cluster_prefs,
    },
];

/// The per-cluster settings (`clusters.<id>`) follow into the session manager (E06-S08) and the
/// connections' watch budgets (E04-F543) for the life of the app.
fn follow_cluster_prefs(cx: &mut App) {
    if let Some(state) = crate::app_state::AppState::try_global(cx) {
        let sessions = state.services().sessions.clone();
        crate::cluster_prefs::follow_cluster_settings(cx, sessions).detach();
        let budgets = state.ports().clusters.budgets.clone();
        crate::cluster_prefs::follow_watch_budgets(cx, budgets).detach();
    }
}

/// Runs each of `features` in order, timing the whole stage as [`Stage::Features`] and logging
/// each crate's own cost.
pub fn run(cx: &mut App, report: &mut StartupReport, features: &[Feature]) {
    report.time(Stage::Features, || {
        for feature in features {
            let started = std::time::Instant::now();
            (feature.init)(cx);
            tracing::debug!(
                feature = feature.name,
                elapsed_us = started.elapsed().as_micros() as u64,
                "feature init"
            );
        }
    });
}
