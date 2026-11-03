//! The controls on the page, over a settings store installed from a temporary directory.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AppContext as _, Entity, Modifiers, TestAppContext, UpdateGlobal as _, VisualTestContext,
};
use oxikube_settings::SettingsStore;
use oxikube_workspace::{OpenOptions, test_support::open_workspace};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::model;
use crate::controls::WRITE_DEBOUNCE;
use crate::page::SettingsPage;
use crate::page::test_support::install_settings;

struct Harness {
    vcx: VisualTestContext,
    page: Entity<SettingsPage>,
    config: TempDir,
}

impl Harness {
    /// The page open on the `terminal` section, over `user_settings`.
    fn open(cx: &mut TestAppContext, user_settings: &str, section: &str) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        let config = tempfile::tempdir().expect("a temporary config directory");
        let mut page = None;
        vcx.update(|window, cx| {
            install_settings(config.path(), user_settings, cx);
            let entity = cx.new(|cx| SettingsPage::with_model(model(), window, cx));
            page = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        vcx.run_until_parked();
        let mut harness = Self {
            vcx,
            page: page.expect("the page was built"),
            config,
        };
        if section != "general" {
            harness.click(&format!("settings-nav-{section}"));
        }
        harness
    }

    fn click(&mut self, selector: &str) {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let at = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center();
        self.vcx.simulate_click(at, Modifiers::none());
        self.vcx.run_until_parked();
    }

    fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector).is_some()
    }

    /// Types `text` into the input laid out at `selector`, replacing its content, as a person
    /// would: click to focus, select all, type.
    fn type_at(&mut self, selector: &str, text: &str) {
        self.click(selector);
        self.vcx.simulate_keystrokes("cmd-a backspace");
        if !text.is_empty() {
            self.vcx.simulate_input(text);
        }
        self.vcx.run_until_parked();
    }

    fn type_into(&mut self, path: &str, text: &str) {
        self.type_at(&format!("settings-input-{path}"), text);
    }

    fn shown_in(&mut self, path: &str) -> String {
        let input = self
            .page
            .read_with(&self.vcx, |page, _| page.controls.input(path).cloned())
            .unwrap_or_else(|| panic!("{path} has no input"));
        self.vcx.read(|cx| input.read(cx).value().to_string())
    }

    fn pause(&mut self, by: Duration) {
        self.vcx.executor().advance_clock(by);
        self.vcx.run_until_parked();
    }

    fn file_path(&self) -> PathBuf {
        self.config.path().join("settings.json")
    }

    fn file(&self) -> String {
        std::fs::read_to_string(self.file_path()).expect("settings.json exists")
    }

    /// The user file as JSON (comments and commas stripped by the lenient parser).
    fn file_json(&self) -> Value {
        let object =
            oxikube_settings::jsonc::parse_jsonc_object(&self.file()).expect("valid JSONC");
        Value::Object(object)
    }
}

#[gpui::test]
fn a_switch_writes_at_once_and_keeps_comments(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{\n  // my notes\n  \"ui_scale\": 1.25\n}", "general");
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
    let mut page = Harness::open(cx, "{}", "terminal");
    page.click("settings-choice-terminal.cursor_shape-bar");
    assert_eq!(page.file_json()["terminal"]["cursor_shape"], json!("bar"));
}

#[gpui::test]
fn a_number_is_written_after_the_pause_not_per_keystroke(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}", "terminal");
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
    let mut page = Harness::open(cx, r#"{ "terminal": { "font_size": 15 } }"#, "terminal");
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
    let mut page = Harness::open(cx, "{}", "terminal");
    page.type_into("terminal.font_size", "20");
    page.type_into("terminal.font_size", "999");
    page.pause(WRITE_DEBOUNCE * 2);
    assert_eq!(page.file().trim(), "{}");
}

#[gpui::test]
fn reset_removes_only_that_key_and_shows_only_where_the_user_set_one(cx: &mut TestAppContext) {
    let user = "{\n  // keep me\n  \"terminal\": { \"font_size\": 15, \"shell\": \"zsh\" }\n}";
    let mut page = Harness::open(cx, user, "terminal");
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
    let mut page = Harness::open(cx, r#"{ "terminal": { "shell": "zsh" } }"#, "terminal");
    page.type_into("terminal.shell", "");
    page.pause(WRITE_DEBOUNCE);
    assert_eq!(page.file_json().get("terminal"), None, "{}", page.file());
}

#[gpui::test]
fn a_colour_is_normalised_and_drawn_as_a_swatch(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}", "terminal");
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
    let mut page = Harness::open(
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
        page.file_json().get("terminal"),
        None,
        "an empty list resets: {}",
        page.file()
    );
}

#[gpui::test]
fn a_shape_without_a_control_is_read_only(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}", "terminal");
    assert!(page.is_laid_out("settings-value-terminal.env"));
    assert!(!page.is_laid_out("settings-input-terminal.env"));
}

#[gpui::test]
fn an_unfocused_input_follows_the_file_and_a_pending_one_does_not(cx: &mut TestAppContext) {
    let mut page = Harness::open(
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
    let mut page = Harness::open(cx, "{}", "terminal");
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

impl Harness {
    fn read_controls<R>(&mut self, f: impl FnOnce(&crate::controls::Controls) -> R) -> R {
        self.page.read_with(&self.vcx, |page, _| f(&page.controls))
    }
}
