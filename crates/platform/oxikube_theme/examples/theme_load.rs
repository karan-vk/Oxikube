//! Startup and switch cost of themes: the cold work `init` does on the main thread, importing a
//! user theme family, and resolving the active theme. Reports the median and worst of 200 runs
//! (the very first run is reported separately: it includes the one-time fallback parse).
//!
//! `cargo run --release -p oxikube_theme --example theme_load` (budget: settings + keymap + theme
//! < 30 ms combined on the main thread at startup, E05-S13; a theme switch is one `resolve` and
//! one `ActiveTheme` write).
#![allow(clippy::print_stdout)]

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
    (times[times.len() / 2], *times.last().unwrap())
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
        measure(200, || drop(import_family(AYU).unwrap())),
    );
    report(
        "import Gruvbox (6 themes, 82 KB)",
        measure(200, || drop(import_family(GRUVBOX).unwrap())),
    );

    // Starting the hot-reload watcher is the only syscall-heavy step `init` does on the main
    // thread (mkdir + registering the watch); the scan itself runs on the watcher thread.
    let dir = std::env::temp_dir().join(format!("oxikube-theme-load-{}", std::process::id()));
    report(
        "ThemeDirWatcher::spawn (mkdir + watch)",
        measure(20, || {
            drop(ThemeDirWatcher::spawn(dir.join("themes"), DEFAULT_DEBOUNCE, |_| true).unwrap());
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
}
