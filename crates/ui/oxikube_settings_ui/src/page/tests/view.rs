//! The page in a workspace window: navigation, search, jumping to a result, layer badges.

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_settings::{SettingSource, SettingsStore};

use super::Harness;

const USER: &str = r#"{
  // the user's own font size
  "terminal": { "font_size": 15 },
  "ui_scale": 1.25,
}"#;

#[gpui::test]
fn the_page_lists_every_section_and_draws_only_the_selected_one(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    for key in ["general", "editor", "terminal"] {
        assert!(
            page.is_laid_out(&format!("settings-nav-{key}")),
            "{key} in the nav"
        );
    }
    assert_eq!(page.read(|p| p.selected_section()), 0);
    assert!(page.is_laid_out("settings-field-ui_scale"));
    assert!(
        !page.is_laid_out("settings-field-terminal.font_size"),
        "a section that is not selected is not built"
    );
}

#[gpui::test]
fn the_navigation_switches_sections(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    page.click("settings-nav-terminal");
    assert_eq!(page.read(|p| p.selected_section()), 2);
    assert!(page.is_laid_out("settings-field-terminal.font_size"));
    assert!(!page.is_laid_out("settings-field-ui_scale"));

    page.click("settings-nav-editor");
    assert_eq!(page.read(|p| p.selected_section()), 1);
    assert!(page.is_laid_out("settings-field-editor.tab_size"));
}

#[gpui::test]
fn search_finds_a_field_by_its_description_and_jumps_to_it(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    // The page gives its focus to the search field: typing searches at once.
    page.type_text("clipboard");
    assert_eq!(page.read(|p| p.query().to_owned()), "clipboard");
    assert_eq!(page.read(|p| p.hits().len()), 1);
    assert!(page.is_laid_out("settings-hit-0"));
    assert!(page.is_laid_out("settings-results-summary"));
    assert!(
        !page.is_laid_out("settings-field-ui_scale"),
        "results replace the section"
    );

    page.click("settings-hit-0");
    let (section, highlighted, query) = page.read(|p| {
        (
            p.selected_section(),
            p.highlighted_field(),
            p.query().to_owned(),
        )
    });
    assert_eq!(section, 2, "the Terminal section");
    let (_, copy_on_select) = page
        .read(|p| p.model().find_field("terminal.copy_on_select"))
        .expect("the field");
    assert_eq!(highlighted, Some(copy_on_select));
    assert!(query.is_empty(), "the search is cleared");
    let field = page
        .bounds("settings-field-terminal.copy_on_select")
        .expect("the field is drawn");
    let content = page.bounds("settings-content").expect("the content");
    assert!(
        field.top() >= content.top() && field.bottom() <= content.bottom(),
        "scrolled into view: {field:?} in {content:?}"
    );
}

#[gpui::test]
fn enter_opens_the_best_result(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    page.type_text("tab size");
    page.keys("enter");
    assert_eq!(page.read(|p| p.selected_section()), 1, "the editor section");
    assert!(page.is_laid_out("settings-field-editor.tab_size"));
}

#[gpui::test]
fn a_query_with_no_match_says_so(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    page.type_text("zqxj");
    assert!(page.read(|p| p.hits().is_empty()));
    assert!(page.is_laid_out("settings-results-summary"));
}

#[gpui::test]
fn each_field_shows_the_layer_its_value_comes_from(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, USER);
    assert!(page.is_laid_out("settings-badge-ui_scale-User"));
    assert!(page.is_laid_out("settings-badge-confirm_quit-Default"));
    page.click("settings-nav-terminal");
    assert!(page.is_laid_out("settings-badge-terminal.font_size-User"));
    assert!(page.is_laid_out("settings-badge-terminal.cursor_blink-Default"));
    let source = page.vcx.update(|_, cx| {
        let page = page.page.read(cx);
        page.source_of("terminal.font_size", cx)
    });
    assert_eq!(source, SettingSource::User);
}

#[gpui::test]
fn the_badges_follow_a_settings_change(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}");
    assert!(page.is_laid_out("settings-badge-ui_scale-Default"));
    page.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{ "ui_scale": 1.5 }"#)
                .expect("valid settings");
        });
    });
    page.vcx.run_until_parked();
    assert!(page.is_laid_out("settings-badge-ui_scale-User"));
    assert!(!page.is_laid_out("settings-badge-ui_scale-Default"));
}
