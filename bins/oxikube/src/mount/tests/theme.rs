//! The `theme` setting in the real app (E22-S07): an unknown theme name is one toast, and the
//! window follows the operating system's appearance (as the first window only).

use std::path::Path;

use gpui::TestAppContext;
use oxikube_testkit::TestPorts;
use oxikube_theme::{ActiveTheme, Appearance, SystemAppearance};

use super::App;
use crate::startup::ConfigSource;

fn start(cx: &mut TestAppContext, dir: &Path) -> App {
    let dir = dir.to_owned();
    App::start_with_env(
        cx,
        TestPorts::seeded(),
        move |env| env.config = ConfigSource::Dir(dir),
        |_| {},
    )
}

fn toast_messages(app: &mut App) -> Vec<String> {
    app.read(|ws, cx| {
        ws.toast_layer()
            .read(cx)
            .visible()
            .iter()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

#[gpui::test]
fn an_unknown_theme_name_is_one_toast_and_the_default_theme_shows(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{ "theme": { "mode": "dark", "dark": "Nonexistent Night" } }"#,
    )
    .unwrap();
    let mut app = start(cx, dir.path());

    let messages = toast_messages(&mut app);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("Nonexistent Night"), "{}", messages[0]);
    assert!(messages[0].contains("One Dark"), "{}", messages[0]);
    app.vcx
        .update(|_, cx| assert_eq!(ActiveTheme::get(cx).name, "One Dark"));
}

#[gpui::test]
fn a_known_theme_shows_no_toast_and_the_system_appearance_is_followed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = start(cx, dir.path());
    assert!(toast_messages(&mut app).is_empty());

    // The mounted window followed the (test) platform's appearance, which is light.
    app.vcx
        .update(|_, cx| assert_eq!(ActiveTheme::get(cx).name, "One Light"));
    app.vcx
        .update(|_, cx| SystemAppearance::set(cx, Appearance::Dark));
    app.vcx
        .update(|_, cx| assert_eq!(ActiveTheme::get(cx).name, "One Dark"));
    assert!(toast_messages(&mut app).is_empty());
}
