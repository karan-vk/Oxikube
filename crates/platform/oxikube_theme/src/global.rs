//! GPUI wiring: the registry and active theme as globals, kept current by settings, the system
//! appearance and the `themes/` directory.
//!
//! The flow, all on the foreground thread except the directory scan:
//!
//! 1. [`init`] installs a [`ThemeRegistry`] with the bundled themes, resolves the `theme`
//!    setting against the system appearance and sets [`ActiveTheme`] (so the first frame is
//!    already themed), then starts the directory watcher.
//! 2. The watcher thread scans `<config>/themes/` and sends finished scans; a foreground task
//!    swaps each into the registry and re-resolves.
//! 3. A change of the `theme` setting or of [`SystemAppearance`] re-resolves too.
//!
//! [`ActiveTheme`] is only written when the resolved tokens actually differ, so observers (the
//! `oxikube_ui` bridge) run once per real theme change, never per rescan or unrelated settings
//! edit.

use crate::appearance::SystemAppearance;
use crate::registry::ThemeRegistry;
use crate::settings::{ThemeSelection, ThemeSettings};
use crate::tokens::ThemeTokens;
use crate::user_dir::{UserThemes, scan_dir, themes_dir};
use crate::watcher::{DEFAULT_DEBOUNCE, ThemeDirWatcher};
use futures::StreamExt as _;
use gpui::{App, BorrowAppContext as _, Global, Task};
use oxikube_settings::Settings as _;
use std::path::Path;
use std::sync::Arc;

impl Global for ThemeRegistry {}

/// The theme currently in effect. Observe it with `cx.observe_global::<ActiveTheme>(..)`.
#[derive(Clone, Debug)]
pub struct ActiveTheme(pub Arc<ThemeTokens>);

impl Global for ActiveTheme {}

impl ActiveTheme {
    /// The active theme; the bundled fallback for the system appearance before [`init`].
    pub fn get(cx: &App) -> Arc<ThemeTokens> {
        match cx.try_global::<Self>() {
            Some(active) => active.0.clone(),
            None => Arc::new(ThemeTokens::fallback(SystemAppearance::get(cx)).clone()),
        }
    }
}

impl ThemeRegistry {
    /// The registry installed by [`init`]. Panics before it (a wiring bug).
    #[track_caller]
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// The registry, or `None` before [`init`].
    pub fn try_global(cx: &App) -> Option<&Self> {
        cx.try_global::<Self>()
    }
}

/// Keeps the watcher thread and its apply task alive for the life of the app.
struct ThemeWatch {
    _watcher: ThemeDirWatcher,
    _apply: Task<()>,
}

impl Global for ThemeWatch {}

/// Installs themes from the default config dir (`<config>/themes/`, see
/// `oxikube_settings::paths::config_dir`) and starts hot reload. Idempotent.
///
/// Call after `oxikube_settings::init` so the `theme` setting is read; without a settings store
/// the default selection (follow the system, One Light / One Dark) applies.
pub fn init(cx: &mut App) {
    let dir = oxikube_settings::paths::config_dir().map(|dir| themes_dir(&dir));
    install(cx, dir.as_deref(), true);
}

/// Like [`init`] with an explicit themes directory and no hot reload thread, scanning the
/// directory once on the calling thread. For tests and tools.
pub fn init_with_dir(themes_dir: Option<&Path>, cx: &mut App) {
    install(cx, themes_dir, false);
}

/// Like [`init_with_dir`] but with the hot-reload watcher on `themes_dir` (created when
/// missing). The watcher is an OS thread, so a test using it must call
/// `cx.executor().allow_parking()` first; use it in the one test that exercises hot reload.
pub fn init_watching_dir(themes_dir: &Path, cx: &mut App) {
    install(cx, Some(themes_dir), true);
}

fn install(cx: &mut App, dir: Option<&Path>, watch: bool) {
    if cx.has_global::<ThemeRegistry>() {
        return;
    }
    SystemAppearance::init(cx);
    cx.set_global(ThemeRegistry::with_bundled());
    if cx.has_global::<oxikube_settings::SettingsStore>() {
        ThemeSettings::register(cx);
        ThemeSettings::observe(cx, refresh_active).detach();
    }
    cx.observe_global::<SystemAppearance>(refresh_active)
        .detach();
    refresh_active(cx);

    let Some(dir) = dir else {
        return;
    };
    if watch {
        start_watch(cx, dir.to_path_buf());
    } else {
        apply_user_scan(cx, scan_dir(dir));
    }
}

fn start_watch(cx: &mut App, dir: std::path::PathBuf) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<UserThemes>();
    let watcher = match ThemeDirWatcher::spawn(dir, DEFAULT_DEBOUNCE, move |scan| {
        tx.unbounded_send(scan).is_ok()
    }) {
        Ok(watcher) => watcher,
        Err(err) => {
            tracing::warn!(%err, "theme hot reload is off");
            return;
        }
    };
    let apply = cx.spawn(async move |cx| {
        while let Some(scan) = rx.next().await {
            cx.update(|cx| apply_user_scan(cx, scan));
        }
    });
    cx.set_global(ThemeWatch {
        _watcher: watcher,
        _apply: apply,
    });
}

/// Replaces the registry's user themes with `scan` and re-resolves the active theme.
///
/// What the watcher's apply task does for each finished scan; public so a host that scans
/// itself (or a test with a scripted scan) can feed the registry.
pub fn apply_user_scan(cx: &mut App, scan: UserThemes) {
    for problem in &scan.problems {
        tracing::warn!(path = %problem.path.display(), message = %problem.message, "theme file");
    }
    cx.update_global::<ThemeRegistry, _>(|registry, _| registry.replace_user_themes(scan));
    refresh_active(cx);
}

/// Resolves the `theme` setting against the system appearance and sets [`ActiveTheme`] when the
/// result differs from the current one.
pub fn refresh_active(cx: &mut App) {
    let selection = ThemeSettings::try_get(cx)
        .map(|settings| settings.selection.clone())
        .unwrap_or_default();
    let resolved = resolve(cx, &selection);
    let unchanged = cx
        .try_global::<ActiveTheme>()
        .is_some_and(|active| Arc::ptr_eq(&active.0, &resolved) || *active.0 == *resolved);
    if !unchanged {
        cx.set_global(ActiveTheme(resolved));
    }
}

fn resolve(cx: &App, selection: &ThemeSelection) -> Arc<ThemeTokens> {
    cx.global::<ThemeRegistry>()
        .resolve(selection, SystemAppearance::get(cx))
}
