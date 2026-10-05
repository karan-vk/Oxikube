//! The Linux desktop assets match the application id the window sets.
//!
//! Wayland compositors find the icon and group windows by matching the window's `app_id` to the
//! desktop file name; X11 uses `StartupWMClass`. A mismatch silently loses the icon, so the id is
//! pinned here against `oxikube_workspace::window::APP_ID`.

use oxikube_workspace::window::APP_ID;
use std::path::Path;

fn resources() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/linux")
}

fn desktop_entry() -> String {
    let path = resources().join(format!("{APP_ID}.desktop"));
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "desktop file {} (named after the app id): {err}",
            path.display()
        )
    })
}

fn key<'a>(entry: &'a str, name: &str) -> Option<&'a str> {
    entry
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
}

#[test]
fn desktop_file_is_named_after_the_app_id_and_matches_its_wm_class() {
    let entry = desktop_entry();
    assert!(entry.starts_with("[Desktop Entry]"));
    assert_eq!(key(&entry, "StartupWMClass"), Some(APP_ID));
    assert_eq!(key(&entry, "Icon"), Some(APP_ID));
    assert_eq!(key(&entry, "Type"), Some("Application"));
    assert_eq!(key(&entry, "Exec"), Some("oxikube %U"));
}

#[test]
fn the_named_icon_ships() {
    assert!(resources().join(format!("{APP_ID}.svg")).is_file());
}
