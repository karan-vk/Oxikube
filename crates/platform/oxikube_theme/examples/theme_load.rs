//! Startup and switch cost of themes: the cold work `init` does on the main thread, importing a
//! user theme family, and resolving the active theme. Reports the median and worst of 200 runs
//! (the very first run is reported separately: it includes the one-time fallback parse).
//!
//! `cargo run --release -p oxikube_theme --example theme_load` (budget: settings + keymap + theme
//! < 30 ms combined on the main thread at startup, E05-S13; a theme switch is one `resolve` and
//! one `ActiveTheme` write). Icon themes (E22-S11): starting the `icon_themes/` watcher, reading
//! and mounting a family, building its resolver (once per icon-theme switch) and the per-icon
//! lookups drawing does.
#![allow(clippy::print_stdout, reason = "an example reports on stdout")]

use oxikube_theme::icons::{IconResolver, IconThemeFamily, read_icon_theme_file};
use oxikube_theme::watcher::{DEFAULT_DEBOUNCE, ThemeDirWatcher};
use oxikube_theme::{Appearance, ThemeMode, ThemeRegistry, ThemeSelection, import_family};
use std::time::{Duration, Instant};

const AYU: &str = include_str!("../tests/fixtures/ayu.json");
const GRUVBOX: &str = include_str!("../tests/fixtures/gruvbox.json");

fn measure(runs: usize, mut work: impl FnMut()) -> (Duration, Duration) {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed()
        })
        .collect();
    times.sort();
    (
        times[times.len() / 2],
        *times.last().expect("measure runs at least once"),
    )
}

fn report(label: &str, (median, worst): (Duration, Duration)) {
    println!("{label:<46} median {median:>10.2?}   worst {worst:>10.2?}");
}

fn main() {
    let start = Instant::now();
    let registry = ThemeRegistry::with_bundled();
    println!(
        "first ThemeRegistry::with_bundled (cold)       {:>10.2?}",
        start.elapsed()
    );

    report(
        "ThemeRegistry::with_bundled (warm fallbacks)",
        measure(200, || drop(ThemeRegistry::with_bundled())),
    );
    report(
        "import Ayu (3 themes, 39 KB)",
        measure(200, || {
            drop(import_family(AYU).expect("bundled Ayu fixture imports"))
        }),
    );
    report(
        "import Gruvbox (6 themes, 82 KB)",
        measure(200, || {
            drop(import_family(GRUVBOX).expect("bundled Gruvbox fixture imports"))
        }),
    );

    // Starting the hot-reload watcher is the only syscall-heavy step `init` does on the main
    // thread (mkdir + registering the watch); the scan itself runs on the watcher thread.
    let dir = std::env::temp_dir().join(format!("oxikube-theme-load-{}", std::process::id()));
    report(
        "ThemeDirWatcher::spawn (mkdir + watch)",
        measure(20, || {
            drop(
                ThemeDirWatcher::spawn(dir.join("themes"), DEFAULT_DEBOUNCE, |_| true)
                    .expect("theme watcher starts on a temp dir"),
            );
        }),
    );
    let _ = std::fs::remove_dir_all(&dir);

    let selection = ThemeSelection::Dynamic {
        mode: ThemeMode::System,
        light: "One Light".into(),
        dark: "One Dark".into(),
    };
    report(
        "resolve(selection, system) (a theme switch)",
        measure(200, || drop(registry.resolve(&selection, Appearance::Dark))),
    );

    icon_themes(&dir);
}

fn icon_themes(dir: &std::path::Path) {
    report(
        "icon watcher spawn_scanning (mkdir + watch)",
        measure(20, || {
            drop(
                ThemeDirWatcher::spawn_scanning(
                    dir.join("icon_themes"),
                    DEFAULT_DEBOUNCE,
                    notify::RecursiveMode::Recursive,
                    oxikube_theme::icons::scan_icon_themes_dir,
                    |_| true,
                )
                .expect("icon theme watcher starts on a temp dir"),
            );
        }),
    );
    let _ = std::fs::remove_dir_all(dir);

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icon_themes");
    let read = || read_icon_theme_file(&root.join("kube.json"), &root).expect("fixture reads");
    report(
        "read kube icon theme (7 SVGs, watcher thread)",
        measure(200, || drop(read())),
    );
    let source = read();
    report(
        "mount it (UI thread)",
        measure(200, || drop(IconThemeFamily::mount(source.clone()))),
    );
    let theme = std::sync::Arc::new(IconThemeFamily::mount(read()).themes.remove(0));
    report(
        "IconResolver::new (an icon-theme switch)",
        measure(200, || drop(IconResolver::new(theme.clone()))),
    );
    let icons = IconResolver::new(theme);
    const LOOKUPS: u32 = 10_000;
    let (median, worst) = measure(50, || {
        for _ in 0..LOOKUPS {
            std::hint::black_box(icons.kind(std::hint::black_box(""), "Pod"));
            std::hint::black_box(icons.status(std::hint::black_box("Pending")));
        }
    });
    report(
        "10k kind + 10k status lookups (a 10k-row table)",
        (median, worst),
    );
}
