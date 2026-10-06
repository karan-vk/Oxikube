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

/// The feature crates, in the order their `init`s run. Empty until the first feature epic lands
/// (the palette, resources table and others have no `init` yet).
pub const FEATURES: &[Feature] = &[];

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
