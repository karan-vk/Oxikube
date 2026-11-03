//! GPUI wiring of icon themes: the [`ActiveIconTheme`] global, kept current by the `icon_theme`
//! setting, the system appearance and the `icon_themes/` directory, and the per-thread copy of
//! its [`IconResolver`] that drawing reads.
//!
//! Why a thread-local copy: views draw icons through `oxikube_ui::Icon`, which also converts
//! into the component library's icon where no `App` is at hand (`Button::icon(..)`). Every GPUI
//! window draws on the app's one foreground thread, so the resolver is published there whenever
//! [`ActiveIconTheme`] changes, and read there with [`icon_path`] and friends: a borrow and a
//! reference-count bump, no lock, no allocation. A thread that never had an icon theme (a
//! background thread, a test without [`init_icon_themes_with_dir`]) gets Lucide.

use super::read::IconThemeSource;
use super::resolver::IconResolver;
use super::settings::IconThemeSettings;
use super::theme::IconThemeFamily;
use super::user_dir::{UserIconThemes, icon_themes_dir, scan_icon_themes_dir};
use crate::appearance::SystemAppearance;
use crate::registry::{ThemeRegistry, ThemeSource};
use crate::watcher::{DEFAULT_DEBOUNCE, ThemeDirWatcher};
use futures::StreamExt as _;
use gpui::{App, BorrowAppContext as _, Global, SharedString, Task};
use notify::RecursiveMode;
use oxikube_assets::IconName;
use oxikube_settings::Settings as _;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

/// The icon theme currently in effect, resolved. Observe it with
/// `cx.observe_global::<ActiveIconTheme>(..)`.
#[derive(Clone, Debug)]
pub struct ActiveIconTheme(pub Arc<IconResolver>);

impl Global for ActiveIconTheme {}

impl ActiveIconTheme {
    /// The active resolver; Lucide before [`init_icon_themes`].
    pub fn get(cx: &App) -> Arc<IconResolver> {
        match cx.try_global::<Self>() {
            Some(active) => active.0.clone(),
            None => LUCIDE.clone(),
        }
    }
}

static LUCIDE: LazyLock<Arc<IconResolver>> = LazyLock::new(|| Arc::new(IconResolver::lucide()));

thread_local! {
    static CURRENT: RefCell<Option<Arc<IconResolver>>> = const { RefCell::new(None) };
}

/// Runs `f` with this thread's active resolver (Lucide when none was published here).
pub fn with_active_icons<R>(f: impl FnOnce(&IconResolver) -> R) -> R {
    CURRENT.with_borrow(|current| f(current.as_deref().unwrap_or(&LUCIDE)))
}

/// The asset path `icon` draws from under the active icon theme.
pub fn icon_path(icon: IconName) -> SharedString {
    with_active_icons(|icons| icons.icon(icon))
}

/// The asset path of the icon of `kind` in API `group` (empty for core) under the active icon
/// theme.
pub fn kind_icon_path(group: &str, kind: &str) -> SharedString {
    with_active_icons(|icons| icons.kind(group, kind))
}

/// The asset path of the icon of the status text `status` under the active icon theme.
pub fn status_icon_path(status: &str) -> SharedString {
    with_active_icons(|icons| icons.status(status))
}

/// Keeps the icon-themes watcher thread and its apply task alive.
struct IconThemeWatch {
    _watcher: ThemeDirWatcher,
    _apply: Task<()>,
}

impl Global for IconThemeWatch {}

/// Marks that icon themes were installed in this app.
struct IconThemesInstalled;

impl Global for IconThemesInstalled {}

/// Installs icon themes from `<config>/icon_themes/` with hot reload (a watcher thread does the
/// reading) and follows the `icon_theme` setting. Idempotent; needs [`crate::init`] first (the
/// icon themes live in its [`ThemeRegistry`]).
pub fn init_icon_themes(cx: &mut App) {
    let dir = oxikube_settings::paths::config_dir().map(|dir| icon_themes_dir(&dir));
    install(cx, dir.as_deref(), Watch::Thread);
}

/// Like [`init_icon_themes`] with an explicit directory scanned once on the calling thread, no
/// watcher. For tests and tools; `None` installs only the built-in Lucide theme.
pub fn init_icon_themes_with_dir(dir: Option<&Path>, cx: &mut App) {
    install(cx, dir, Watch::Once);
}

/// How the icon-themes directory is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Watch {
    /// Scanned now, on the calling thread.
    Once,
    /// Scanned and rescanned on change by a watcher thread.
    Thread,
}

fn install(cx: &mut App, dir: Option<&Path>, watch: Watch) {
    if cx.has_global::<IconThemesInstalled>() {
        return;
    }
    if !cx.has_global::<ThemeRegistry>() {
        tracing::error!("icon themes need oxikube_theme::init first");
        return;
    }
    cx.set_global(IconThemesInstalled);
    if cx.has_global::<oxikube_settings::SettingsStore>() {
        IconThemeSettings::register(cx);
        IconThemeSettings::observe(cx, refresh_active_icon_theme).detach();
    }
    cx.observe_global::<SystemAppearance>(refresh_active_icon_theme)
        .detach();
    refresh_active_icon_theme(cx);

    let Some(dir) = dir else {
        return;
    };
    match watch {
        Watch::Once => apply_user_icon_scan(cx, scan_icon_themes_dir(dir)),
        Watch::Thread => start_watch(cx, dir.to_path_buf()),
    }
}

fn start_watch(cx: &mut App, dir: PathBuf) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<UserIconThemes>();
    let spawned = ThemeDirWatcher::spawn_scanning(
        dir,
        DEFAULT_DEBOUNCE,
        // Subfolders too: a theme keeps its SVGs in one, and an edited SVG should show.
        RecursiveMode::Recursive,
        scan_icon_themes_dir,
        move |scan| tx.unbounded_send(scan).is_ok(),
    );
    let watcher = match spawned {
        Ok(watcher) => watcher,
        Err(err) => {
            tracing::warn!(%err, "icon theme hot reload is off");
            return;
        }
    };
    let apply = cx.spawn(async move |cx| {
        while let Some(scan) = rx.next().await {
            cx.update(|cx| apply_user_icon_scan(cx, scan));
        }
    });
    cx.set_global(IconThemeWatch {
        _watcher: watcher,
        _apply: apply,
    });
}

/// The user icon-theme files of the last applied scan, each with the family mounted from it, so
/// a rescan reuses the mount of a file that did not change.
///
/// Why: every mount serves its files under a fresh `icon-themes/<id>/` prefix, and GPUI's sprite
/// atlas is keyed by path and never evicts SVG tiles. Remounting unchanged files on every rescan
/// (any save in `icon_themes/`) would re-rasterise every visible icon in one frame and leak the
/// old tiles for the life of the window.
struct MountedUserIconThemes(Vec<(PathBuf, IconThemeSource, IconThemeFamily)>);

impl Global for MountedUserIconThemes {}

/// Replaces the registry's user icon themes with `scan` (mounting the files of new or changed
/// families, reusing the mount of unchanged ones) and re-resolves the active icon theme. Public
/// so a host that scans itself, or a test, can feed the registry.
pub fn apply_user_icon_scan(cx: &mut App, scan: UserIconThemes) {
    for problem in &scan.problems {
        tracing::warn!(path = %problem.path.display(), message = %problem.message, "icon theme");
    }
    let mut previous = if cx.has_global::<MountedUserIconThemes>() {
        cx.remove_global::<MountedUserIconThemes>().0
    } else {
        Vec::new()
    };
    let mounted: Vec<(PathBuf, IconThemeSource, IconThemeFamily)> = scan
        .families
        .into_iter()
        .map(|(path, source)| {
            let unchanged = previous
                .iter()
                .position(|(old_path, old_source, _)| *old_path == path && *old_source == source);
            let family = match unchanged {
                Some(index) => previous.swap_remove(index).2,
                None => IconThemeFamily::mount(source.clone()),
            };
            (path, source, family)
        })
        .collect();
    let families = mounted
        .iter()
        .map(|(path, _, family)| (path.clone(), family.clone()))
        .collect();
    cx.set_global(MountedUserIconThemes(mounted));
    cx.update_global::<ThemeRegistry, _>(|registry, _| {
        registry.replace_user_icon_themes(families, scan.problems);
    });
    refresh_active_icon_theme(cx);
}

/// Mounts `read` (an icon-theme family read off the UI thread) under `source` and re-resolves
/// the active icon theme; returns the names added. The extension loader's entry point (E23-S06).
pub fn load_icon_theme(cx: &mut App, source: &ThemeSource, read: IconThemeSource) -> Vec<String> {
    let names =
        cx.update_global::<ThemeRegistry, _>(|registry, _| registry.load_icon_theme(source, read));
    refresh_active_icon_theme(cx);
    names
}

/// Removes every icon theme `source` contributed and re-resolves the active icon theme (Lucide
/// takes over from a removed one). Returns how many were removed.
pub fn remove_icon_themes(cx: &mut App, source: &ThemeSource) -> usize {
    let removed =
        cx.update_global::<ThemeRegistry, _>(|registry, _| registry.remove_icon_themes(source));
    refresh_active_icon_theme(cx);
    removed
}

/// Resolves the `icon_theme` setting and, when the theme differs from the active one, rebuilds
/// the resolver, publishes it to this thread and redraws the windows.
pub fn refresh_active_icon_theme(cx: &mut App) {
    let selection = IconThemeSettings::try_get(cx)
        .map(|settings| settings.selection.clone())
        .unwrap_or_else(|| IconThemeSettings::default().selection);
    let Some(registry) = cx.try_global::<ThemeRegistry>() else {
        return;
    };
    let theme = registry.resolve_icon_theme(&selection, SystemAppearance::get(cx));
    let unchanged = cx
        .try_global::<ActiveIconTheme>()
        .is_some_and(|active| Arc::ptr_eq(active.0.theme(), &theme));
    if unchanged {
        return;
    }
    let resolver = Arc::new(IconResolver::new(theme));
    CURRENT.set(Some(resolver.clone()));
    cx.set_global(ActiveIconTheme(resolver));
    cx.refresh_windows();
}
