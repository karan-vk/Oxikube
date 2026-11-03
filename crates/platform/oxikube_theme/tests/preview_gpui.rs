//! The preview override and the commit path of the theme picker (E22-S08), without a window.

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_settings::{SettingsStore, init_with_dir as init_settings};
use oxikube_theme::global::{ActiveTheme, init_with_dir};
use oxikube_theme::{
    Appearance, SystemAppearance, clear_preview, commit_theme, is_previewing, preview_theme,
};

const SETTINGS: &str = "{\n  // keep me\n  \"theme\": { \"mode\": \"system\", \"light\": \"One Light\", \"dark\": \"One Dark\" },\n  \"zoom\": 1\n}\n";

struct Harness {
    config: tempfile::TempDir,
}

fn setup(cx: &mut TestAppContext, system: Appearance, user: &str) -> Harness {
    let config = tempfile::tempdir().expect("create temp config dir");
    std::fs::write(config.path().join("settings.json"), user).expect("write settings.json");
    cx.update(|cx| {
        init_settings(config.path(), cx);
        SystemAppearance::init(cx);
        SystemAppearance::set(cx, system);
        init_with_dir(None, cx);
    });
    Harness { config }
}

fn active(cx: &TestAppContext) -> String {
    cx.read(|cx| ActiveTheme::get(cx).name.clone())
}

#[gpui::test]
fn a_preview_replaces_the_active_theme_and_clearing_restores_it(cx: &mut TestAppContext) {
    let _harness = setup(cx, Appearance::Dark, SETTINGS);
    assert_eq!(active(cx), "One Dark");
    assert!(cx.update(|cx| preview_theme(cx, "One Light")));
    assert_eq!(active(cx), "One Light");
    assert!(cx.read(is_previewing));
    cx.update(clear_preview);
    assert_eq!(active(cx), "One Dark");
    assert!(!cx.read(is_previewing));
}

#[gpui::test]
fn an_unknown_name_previews_nothing(cx: &mut TestAppContext) {
    let _harness = setup(cx, Appearance::Dark, SETTINGS);
    assert!(!cx.update(|cx| preview_theme(cx, "No Such Theme")));
    assert_eq!(active(cx), "One Dark");
    assert!(!cx.read(is_previewing));
}

#[gpui::test]
fn a_preview_survives_settings_and_system_changes_and_is_never_written(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Dark, SETTINGS);
    cx.update(|cx| preview_theme(cx, "One Light"));
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Light));
    cx.update(|cx| SystemAppearance::set(cx, Appearance::Dark));
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(&SETTINGS.replace("\"zoom\": 1", "\"zoom\": 2"))
                .expect("settings parse")
        })
    });
    assert_eq!(active(cx), "One Light", "the preview outlives the reloads");
    let on_disk = std::fs::read_to_string(harness.config.path().join("settings.json"))
        .expect("read settings.json");
    assert_eq!(on_disk, SETTINGS, "previewing writes nothing");
    cx.update(clear_preview);
    assert_eq!(active(cx), "One Dark");
}

#[gpui::test]
async fn commit_writes_the_active_slot_keeps_comments_and_ends_the_preview(
    cx: &mut TestAppContext,
) {
    let harness = setup(cx, Appearance::Dark, SETTINGS);
    cx.update(|cx| preview_theme(cx, "One Light"));
    cx.update(|cx| commit_theme(cx, "One Light"))
        .await
        .expect("commit succeeds");
    cx.run_until_parked();
    let text = std::fs::read_to_string(harness.config.path().join("settings.json"))
        .expect("read settings.json");
    assert!(text.contains("// keep me"), "{text}");
    assert!(text.contains("\"zoom\": 1"), "{text}");
    assert!(text.contains("\"dark\": \"One Light\""), "{text}");
    assert!(text.contains("\"light\": \"One Light\""), "{text}");
    assert!(!cx.read(is_previewing));
    assert_eq!(
        active(cx),
        "One Light",
        "the committed theme is now the setting's"
    );
}

#[gpui::test]
async fn commit_to_a_static_setting_writes_the_name(cx: &mut TestAppContext) {
    let harness = setup(cx, Appearance::Light, "{ \"theme\": \"One Light\" }\n");
    cx.update(|cx| commit_theme(cx, "One Dark"))
        .await
        .expect("commit succeeds");
    cx.run_until_parked();
    let text = std::fs::read_to_string(harness.config.path().join("settings.json"))
        .expect("read settings.json");
    assert!(text.contains("\"theme\": \"One Dark\""), "{text}");
    assert_eq!(active(cx), "One Dark");
}
