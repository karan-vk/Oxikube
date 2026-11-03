//! The controls on the page, over a settings store installed from a temporary directory.

use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, UpdateGlobal as _};
use oxikube_settings::SettingsStore;
use serde_json::json;

use super::model;
use crate::controls::WRITE_DEBOUNCE;
use crate::page::harness::Harness;

impl Harness {
    /// The page open on `section` of the controls schema, over `user_settings`.
    fn open_section(cx: &mut TestAppContext, user_settings: &str, section: &str) -> Self {
        let mut harness = Self::open_with(cx, model(), user_settings, |_, _, _| {});
        if section != "general" {
            harness.click(&format!("settings-nav-{section}"));
        }
        harness
    }

    fn shown_in(&mut self, path: &str) -> String {
        let input = self
            .page
            .read_with(&self.vcx, |page, _| page.controls.input(path).cloned())
            .unwrap_or_else(|| panic!("{path} has no input"));
        self.vcx.read(|cx| input.read(cx).value().to_string())
    }
}

#[gpui::test]
fn a_switch_writes_at_once_and_keeps_comments(cx: &mut TestAppContext) {
    let mut page =
        Harness::open_section(cx, "{\n  // my notes\n  \"ui_scale\": 1.25\n}", "general");
    // `confirm_quit` is on by default: the first click turns it off.
    page.click("settings-control-confirm_quit");
    assert_eq!(
        page.file_json()["confirm_quit"],
        json!(false),
        "{}",
        page.file()
    );
    assert!(page.file().contains("// my notes"), "{}", page.file());
    // The switch reads the value back from the store.
    page.click("settings-control-confirm_quit");
    assert_eq!(
        page.file_json()["confirm_quit"],
        json!(true),
        "{}",
        page.file()
    );
}

#[gpui::test]
fn a_choice_writes_the_clicked_value(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, "{}", "terminal");
    page.click("settings-choice-terminal.cursor_shape-bar");
    assert_eq!(page.file_json()["terminal"]["cursor_shape"], json!("bar"));
}

#[gpui::test]
fn a_number_is_written_after_the_pause_not_per_keystroke(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, "{}", "terminal");
    page.type_into("terminal.font_size", "1");
    page.pause(Duration::from_millis(100));
    page.type_into("terminal.font_size", "18");
    page.pause(Duration::from_millis(100));
    page.type_into("terminal.font_size", "20");
    assert_eq!(page.file().trim(), "{}", "nothing is written while typing");
    assert!(page.read_controls(|c| c.has_unsaved("terminal.font_size")));

    page.pause(WRITE_DEBOUNCE);
    assert_eq!(
        page.file_json()["terminal"]["font_size"],
        json!(20.0),
        "{}",
        page.file()
    );
    assert!(!page.read_controls(|c| c.has_unsaved("terminal.font_size")));
}

#[gpui::test]
fn invalid_input_shows_an_error_and_never_reaches_the_file(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, r#"{ "terminal": { "font_size": 15 } }"#, "terminal");
    let before = page.file();
    page.type_into("terminal.font_size", "500");
    page.pause(WRITE_DEBOUNCE * 2);
    assert!(page.is_laid_out("settings-error-terminal.font_size"));
    assert_eq!(
        page.file(),
        before,
        "the last valid value stays in the file"
    );

    page.type_into("terminal.font_size", "twelve");
    page.pause(WRITE_DEBOUNCE * 2);
    assert!(page.is_laid_out("settings-error-terminal.font_size"));
    assert_eq!(page.file(), before);

    page.type_into("terminal.font_size", "16");
    assert!(!page.is_laid_out("settings-error-terminal.font_size"));
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(page.file_json()["terminal"]["font_size"], json!(16.0));
}

#[gpui::test]
fn an_invalid_value_drops_the_one_still_waiting(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, "{}", "terminal");
    page.type_into("terminal.font_size", "20");
    page.type_into("terminal.font_size", "999");
    page.pause(WRITE_DEBOUNCE * 2);
    assert_eq!(page.file().trim(), "{}");
}

#[gpui::test]
fn reset_removes_only_that_key_and_shows_only_where_the_user_set_one(cx: &mut TestAppContext) {
    let user = "{\n  // keep me\n  \"terminal\": { \"font_size\": 15, \"shell\": \"zsh\" }\n}";
    let mut page = Harness::open_section(cx, user, "terminal");
    assert!(page.is_laid_out("settings-reset-terminal.font_size"));
    assert!(
        !page.is_laid_out("settings-reset-terminal.scrollback_lines"),
        "a field on its default has nothing to reset"
    );
    page.click("settings-reset-terminal.font_size");
    let file = page.file_json();
    assert!(
        file["terminal"].get("font_size").is_none(),
        "{}",
        page.file()
    );
    assert_eq!(file["terminal"]["shell"], json!("zsh"));
    assert!(page.file().contains("// keep me"));
    assert!(!page.is_laid_out("settings-reset-terminal.font_size"));
    assert_eq!(
        page.shown_in("terminal.font_size"),
        "",
        "the default shows through"
    );
}

#[gpui::test]
fn clearing_an_input_resets_the_field(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, r#"{ "terminal": { "shell": "zsh" } }"#, "terminal");
    page.type_into("terminal.shell", "");
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(page.file_json().get("terminal"), None, "{}", page.file());
}

#[gpui::test]
fn a_colour_is_normalised_and_drawn_as_a_swatch(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, "{}", "terminal");
    assert!(!page.is_laid_out("settings-swatch-terminal.accent"));
    page.type_into("terminal.accent", "#E5484D");
    assert!(page.is_laid_out("settings-swatch-terminal.accent"));
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(page.file_json()["terminal"]["accent"], json!("#e5484d"));

    page.type_into("terminal.accent", "teal");
    assert!(page.is_laid_out("settings-error-terminal.accent"));
}

#[gpui::test]
fn a_list_adds_removes_and_reorders_rows(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(
        cx,
        r#"{ "terminal": { "exec_shells": ["bash", "sh"] } }"#,
        "terminal",
    );
    assert!(page.is_laid_out("settings-list-item-terminal.exec_shells-1"));

    page.click("settings-list-add-terminal.exec_shells");
    assert!(page.is_laid_out("settings-list-item-terminal.exec_shells-2"));
    assert_eq!(
        page.file_json()["terminal"]["exec_shells"],
        json!(["bash", "sh"]),
        "a blank row writes nothing"
    );
    page.click("settings-list-up-terminal.exec_shells-1");
    assert_eq!(
        page.file_json()["terminal"]["exec_shells"],
        json!(["bash", "sh"]),
        "a list with a blank row is invalid and is not written"
    );
    assert!(page.is_laid_out("settings-error-terminal.exec_shells"));
    page.type_at("settings-list-item-terminal.exec_shells-2", "zsh");
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(
        page.file_json()["terminal"]["exec_shells"],
        json!(["sh", "bash", "zsh"])
    );
    assert!(!page.is_laid_out("settings-error-terminal.exec_shells"));
    page.click("settings-list-remove-terminal.exec_shells-2");

    page.click("settings-list-remove-terminal.exec_shells-0");
    assert_eq!(page.file_json()["terminal"]["exec_shells"], json!(["bash"]));

    page.click("settings-list-remove-terminal.exec_shells-0");
    assert_eq!(
        page.file_json()["terminal"]["exec_shells"],
        json!([]),
        "an empty list is written as []: {}",
        page.file()
    );
}

impl Harness {
    fn list_texts(&mut self, path: &str) -> Vec<String> {
        let page = self.page.clone();
        self.vcx.update(|_, cx| {
            page.read(cx)
                .controls
                .list_rows(path)
                .expect("list editor")
                .texts(cx)
        })
    }

    fn touch_store(&mut self, json: &str) {
        self.vcx.update(|_, cx| {
            SettingsStore::update_global(cx, |store, _| store.set_user_settings(json))
                .expect("valid settings");
        });
        self.vcx.run_until_parked();
    }
}

#[gpui::test]
fn list_rows_survive_the_echo_of_their_own_write(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(
        cx,
        r#"{ "terminal": { "exec_shells": ["bash", "sh"] } }"#,
        "terminal",
    );
    page.type_at("settings-list-item-terminal.exec_shells-1", "sh ");
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(
        page.file_json()["terminal"]["exec_shells"],
        json!(["bash", "sh"])
    );
    assert_eq!(
        page.list_texts("terminal.exec_shells"),
        ["bash", "sh "],
        "the row being typed is not rebuilt"
    );
}

#[gpui::test]
fn a_blank_list_row_survives_a_store_change(cx: &mut TestAppContext) {
    let json = r#"{ "terminal": { "exec_shells": ["bash"] } }"#;
    let mut page = Harness::open_section(cx, json, "terminal");
    page.click("settings-list-add-terminal.exec_shells");
    page.touch_store(json);
    assert_eq!(page.list_texts("terminal.exec_shells"), ["bash", ""]);
}

#[gpui::test]
fn a_shape_without_a_control_is_read_only(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, "{}", "terminal");
    assert!(page.is_laid_out("settings-value-terminal.env"));
    assert!(!page.is_laid_out("settings-input-terminal.env"));
}

#[gpui::test]
fn an_unfocused_input_follows_the_file_and_a_pending_one_does_not(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(
        cx,
        r#"{ "terminal": { "shell": "zsh", "font_size": 15 } }"#,
        "terminal",
    );
    assert_eq!(page.shown_in("terminal.shell"), "zsh");
    page.type_into("terminal.font_size", "20");

    page.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(r#"{ "terminal": { "shell": "fish", "font_size": 15 } }"#)
        })
        .expect("valid settings");
    });
    page.vcx.run_until_parked();
    assert_eq!(page.shown_in("terminal.shell"), "fish", "followed the file");
    assert_eq!(
        page.shown_in("terminal.font_size"),
        "20",
        "a typed value is kept"
    );
}

#[gpui::test]
fn the_page_writes_the_cluster_block_when_it_has_a_cluster(cx: &mut TestAppContext) {
    // The cluster tab (E21-S06) sets the page's cluster; the controls then edit `clusters.<id>`.
    let mut page = Harness::open_section(cx, "{}", "terminal");
    let id =
        oxikube_domain::ids::ClusterId::new("test", &oxikube_domain::ids::ContextName::new("kind"));
    page.page
        .update(&mut page.vcx, |page, _| page.cluster = Some(id.clone()));
    page.click("settings-choice-terminal.cursor_shape-block");
    let clusters = page.file_json()["clusters"].clone();
    assert_eq!(
        clusters[id.as_str()]["terminal"]["cursor_shape"],
        json!("block"),
        "{}",
        page.file()
    );
}

#[gpui::test]
fn the_font_families_are_listed_off_the_ui_thread_once_per_app(cx: &mut TestAppContext) {
    let mut page = Harness::open_section(cx, "{}", "general");
    let fields = page.read(|p| {
        p.model()
            .sections()
            .iter()
            .find(|section| section.key == "editor")
            .expect("the editor section")
            .fields
            .clone()
    });
    let entity = page.page.clone();
    // Showing a font field (what `render` does) only starts the listing: the frame never waits
    // for the platform to walk every font face.
    page.vcx.update(|window, cx| {
        entity.update(cx, |p, cx| {
            p.controls.ensure(&fields, None, window, cx);
            assert!(!p.controls.fonts_listed(), "listed inside the frame");
            assert!(
                p.controls.fonts_task.is_some(),
                "the listing was not started"
            );
        });
    });
    page.vcx.run_until_parked();
    assert!(page.read_controls(|c| c.fonts_listed()), "the list arrives");

    // A page opened later reads the app's list at once, with no second listing.
    let other = page.vcx.update(|window, cx| {
        cx.new(|cx| crate::page::SettingsPage::with_model(model(), window, cx))
    });
    page.vcx.update(|window, cx| {
        other.update(cx, |p, cx| {
            p.controls.ensure(&fields, None, window, cx);
            assert!(p.controls.fonts_listed() && p.controls.fonts_task.is_none());
        });
    });
}

#[test]
fn the_suggested_families_drop_hidden_ones_and_are_sorted_once_each() {
    let names = crate::controls::fonts::suggestable(vec![
        "Menlo".into(),
        ".SF NS".into(),
        "Arial".into(),
        "Menlo".into(),
    ]);
    let names: Vec<&str> = names.iter().map(|name| name.as_ref()).collect();
    assert_eq!(names, ["Arial", "Menlo"]);
}

impl Harness {
    fn read_controls<R>(&mut self, f: impl FnOnce(&crate::controls::Controls) -> R) -> R {
        self.page.read_with(&self.vcx, |page, _| f(&page.controls))
    }
}
