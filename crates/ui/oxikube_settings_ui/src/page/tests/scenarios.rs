//! The consolidated scenarios of the settings page (E21-S10): the ones that guard the user's
//! file, run end to end on the shared [`Harness`] with no watcher and no sleeping (the write
//! debounce is passed with the deterministic clock). What each story already tests in depth
//! (control kinds, the list editor, the cluster tab) stays with that story.

use std::time::Duration;

use gpui::{Focusable as _, TestAppContext, UpdateGlobal as _};
use oxikube_settings::SettingsStore;
use serde_json::json;

use super::Harness;
use crate::controls::WRITE_DEBOUNCE;

const WITH_COMMENTS: &str = "{\n  // before\n  \"ui_scale\": 1.25,\n  // after\n}";

#[gpui::test]
fn the_harness_never_starts_the_file_watcher(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    assert!(
        !page.vcx.update(|_, cx| oxikube_settings::is_watching(cx)),
        "init_with_dir must not start the watcher thread"
    );
}

#[gpui::test]
fn searching_finds_a_field_and_a_miss_shows_the_empty_state(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    page.type_text("blink");
    assert_eq!(page.read(|p| p.hits().len()), 1, "the cursor_blink field");
    assert!(page.is_laid_out("settings-hit-0"));

    page.keys("secondary-a backspace");
    page.type_text("no such setting");
    assert!(page.read(|p| p.hits().is_empty()));
    assert!(!page.is_laid_out("settings-hit-0"));
    assert!(page.is_laid_out("settings-results-summary"));
}

#[gpui::test]
fn toggling_a_switch_changes_only_that_key_and_keeps_both_comments(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, WITH_COMMENTS);
    page.click("settings-control-confirm_quit");
    let file = page.file();
    assert!(file.contains("// before"), "{file}");
    assert!(file.contains("// after"), "{file}");
    assert_eq!(
        page.file_json(),
        json!({ "ui_scale": 1.25, "confirm_quit": false }),
        "{file}"
    );
    let ui_scale_line = |text: &str| {
        text.lines()
            .find(|line| line.contains("ui_scale"))
            .map(str::to_owned)
    };
    assert_eq!(
        ui_scale_line(&file),
        ui_scale_line(WITH_COMMENTS),
        "the untouched key keeps its exact line"
    );
}

#[gpui::test]
fn an_invalid_number_is_rejected_inline_and_a_valid_one_then_writes(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, WITH_COMMENTS);
    page.type_into("ui_scale", "abc");
    page.pause(WRITE_DEBOUNCE * 2);
    assert!(page.is_laid_out("settings-error-ui_scale"));
    assert_eq!(page.file(), WITH_COMMENTS, "the file is untouched");

    page.type_into("ui_scale", "1.5");
    assert!(!page.is_laid_out("settings-error-ui_scale"));
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(page.file_json()["ui_scale"], json!(1.5), "{}", page.file());
    assert!(page.file().contains("// before"), "{}", page.file());
}

#[gpui::test]
fn reset_removes_the_key_and_the_badge_goes_back_to_default(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, WITH_COMMENTS);
    assert!(page.is_laid_out("settings-badge-ui_scale-User"));
    page.click("settings-reset-ui_scale");
    assert_eq!(page.file_json().get("ui_scale"), None, "{}", page.file());
    assert!(page.file().contains("// before"), "{}", page.file());
    assert!(page.is_laid_out("settings-badge-ui_scale-Default"));
    assert!(!page.is_laid_out("settings-badge-ui_scale-User"));
    assert!(!page.is_laid_out("settings-reset-ui_scale"));
}

#[gpui::test]
fn an_external_change_updates_the_controls_and_keeps_focus_and_place(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    page.click("settings-nav-terminal");
    page.click("settings-input-terminal.font_size");
    let focused = |page: &mut Harness| {
        let input = page
            .page
            .read_with(&page.vcx, |p, _| {
                p.controls.input("terminal.font_size").cloned()
            })
            .expect("the input");
        page.vcx
            .update(|window, cx| input.read(cx).focus_handle(cx).is_focused(window))
    };
    assert!(focused(&mut page));
    assert!(page.is_laid_out("settings-badge-terminal.cursor_blink-Default"));

    // The file changed under the page (the watcher's job in the app; the store's manual
    // reload here).
    page.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "terminal": { "cursor_blink": true } }"#)
                .expect("valid settings");
        });
    });
    page.vcx.run_until_parked();
    page.pause(Duration::from_millis(1));

    assert!(page.is_laid_out("settings-badge-terminal.cursor_blink-User"));
    assert_eq!(page.read(|p| p.selected_section()), 2, "still on Terminal");
    assert!(focused(&mut page), "the focus stayed in the input");
}
