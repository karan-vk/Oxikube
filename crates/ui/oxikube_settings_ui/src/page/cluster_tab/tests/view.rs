//! The per-cluster tab on the page: picking a cluster, editing it, the precedence strip.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, TestAppContext};
use oxikube_app::ClusterCatalog;
use oxikube_domain::ClusterColour;
use oxikube_settings::{ClusterSettings, SettingsStore};
use oxikube_testkit::{FakeClockPort, FakeClusterSourcePort, FakeStatePort};
use serde_json::json;

use super::{Harness, entry, id};
use crate::controls::WRITE_DEBOUNCE;
use crate::page::{Scope, SettingsPage};

#[gpui::test]
fn picking_a_cluster_shows_only_the_fields_that_support_cluster_scope(cx: &mut TestAppContext) {
    let mut page = Harness::open(cx, "{}", &["prod", "dev"]);
    assert_eq!(page.page(|p| p.scope()), Scope::Cluster);
    assert_eq!(
        page.page(|p| p.selected_cluster().cloned()),
        Some(id("prod"))
    );
    assert!(page.is_laid_out("settings-field-read_only"));
    assert!(page.is_laid_out("settings-field-display_name"));
    assert!(
        !page.is_laid_out("settings-field-ui_scale"),
        "a global field"
    );
    assert!(page.is_laid_out("settings-nav-prometheus"));
    assert!(!page.is_laid_out("settings-nav-terminal"));

    page.click(&format!("settings-cluster-{}", id("dev")));
    assert_eq!(
        page.page(|p| p.selected_cluster().cloned()),
        Some(id("dev"))
    );

    // Back to all settings: the global sections return.
    page.click("settings-scope-global");
    assert_eq!(page.page(|p| p.scope()), Scope::Global);
    assert!(page.is_laid_out("settings-field-ui_scale"));
    assert!(!page.is_laid_out("settings-field-read_only"));
}

#[gpui::test]
fn an_edit_changes_only_the_cluster_block_and_keeps_the_rest_byte_for_byte(
    cx: &mut TestAppContext,
) {
    let (prod, dev) = (id("prod"), id("dev"));
    let before = format!(
        "{{\n  // my notes\n  \"ui_scale\": 1.25,\n  \"clusters\": {{\n    // dev is mine\n    \"{dev}\": {{ \"read_only\": true }}\n  }}\n}}\n"
    );
    let mut page = Harness::open(cx, &before, &["prod", "dev"]);
    page.reveal("read_only");
    page.click("settings-control-read_only");
    let after = page.file();

    // Everything before the new key is untouched, comments included.
    assert!(
        after.contains("// my notes") && after.contains("// dev is mine"),
        "{after}"
    );
    let (old, new) = (
        oxikube_settings::jsonc::parse_jsonc_object(&before).expect("valid"),
        page.file_json(),
    );
    let new = new.as_object().expect("an object");
    assert_eq!(new["ui_scale"], old["ui_scale"]);
    assert_eq!(new["clusters"][dev.as_str()], old["clusters"][dev.as_str()]);
    assert_eq!(new["clusters"][prod.as_str()], json!({ "read_only": true }));
    // The text outside the new block is the old text, line for line.
    for line in before.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            after.contains(line.trim_end_matches(',')),
            "lost {line:?} in\n{after}"
        );
    }
    // No other cluster and no top-level key was written.
    assert_eq!(new.len(), old.len());
}

#[gpui::test]
fn resetting_removes_the_key_and_the_empty_block(cx: &mut TestAppContext) {
    let (prod, dev) = (id("prod"), id("dev"));
    let before = format!(
        "{{ \"clusters\": {{ \"{prod}\": {{ \"read_only\": true }}, \"{dev}\": {{ \"read_only\": true, \"colour\": \"red\" }} }} }}"
    );
    let mut page = Harness::open(cx, &before, &["prod", "dev"]);
    assert!(page.is_laid_out("settings-reset-read_only"));
    page.reveal("read_only");
    page.click("settings-reset-read_only");
    let after = page.file_json();
    assert!(
        after["clusters"].get(prod.as_str()).is_none(),
        "the emptied block goes: {after}"
    );
    assert_eq!(
        after["clusters"][dev.as_str()],
        json!({ "read_only": true, "colour": "red" }),
        "another cluster is untouched"
    );

    // A block with other keys keeps them.
    page.scroll_to_top();
    page.click(&format!("settings-cluster-{dev}"));
    page.reveal("read_only");
    page.click("settings-reset-read_only");
    assert_eq!(
        page.file_json()["clusters"][dev.as_str()],
        json!({ "colour": "red" })
    );
}

#[gpui::test]
fn the_strip_shows_which_layer_wins(cx: &mut TestAppContext) {
    let (prod, dev) = (id("prod"), id("dev"));
    let user = format!(
        "{{ \"default_namespace\": \"shared\", \"clusters\": {{ \"{prod}\": {{ \"terminal_cwd\": \"/srv\", \"default_namespace\": \"payments\" }} }} }}"
    );
    let mut page = Harness::open(cx, &user, &["prod", "dev"]);
    assert!(page.is_laid_out("settings-precedence-default_namespace"));
    assert!(page.is_laid_out("settings-layer-default_namespace-Cluster-winner"));
    assert!(page.is_laid_out("settings-layer-terminal_cwd-Cluster-winner"));
    assert!(page.is_laid_out("settings-layer-read_only-Default-winner"));
    assert!(page.is_laid_out("settings-precedence-explanation"));

    page.scroll_to_top();
    page.click(&format!("settings-cluster-{dev}"));
    assert!(page.is_laid_out("settings-layer-default_namespace-User-winner"));
    assert!(!page.is_laid_out("settings-layer-default_namespace-Cluster-winner"));
    assert!(page.is_laid_out("settings-layer-terminal_cwd-Default-winner"));
}

#[gpui::test]
fn an_override_that_weakens_a_protection_warns_without_blocking(cx: &mut TestAppContext) {
    let prod = id("prod");
    let user = format!(
        "{{ \"read_only\": true, \"clusters\": {{ \"{prod}\": {{ \"read_only\": true }} }} }}"
    );
    let mut page = Harness::open(cx, &user, &["prod"]);
    assert!(!page.is_laid_out("settings-warning-read_only"));
    // Turning the switch off for this cluster while it is on for the others is allowed...
    page.reveal("read_only");
    page.click("settings-control-read_only");
    assert_eq!(
        page.file_json()["clusters"][prod.as_str()]["read_only"],
        json!(false)
    );
    // ...and says what it does.
    assert!(page.is_laid_out("settings-warning-read_only"));
}

#[gpui::test]
fn a_cluster_edit_reaches_a_consumer_of_the_settings_live(cx: &mut TestAppContext) {
    let (prod, dev) = (id("prod"), id("dev"));
    let mut page = Harness::open(cx, "{}", &["prod", "dev"]);
    // A fake tab: reads the cluster's colour whenever the settings change.
    let seen: Rc<RefCell<Vec<Option<ClusterColour>>>> = Rc::default();
    let sink = seen.clone();
    let tab = prod.clone();
    page.vcx.update(|_, cx| {
        let subscription = cx.observe_global::<SettingsStore>(move |cx| {
            sink.borrow_mut()
                .push(ClusterSettings::resolve(&tab, cx).colour);
        });
        subscription.detach();
    });
    page.type_into("colour", "#e5484d");
    page.pause(WRITE_DEBOUNCE);
    let latest = seen.borrow().last().cloned().flatten();
    assert_eq!(
        latest,
        ClusterColour::parse("#e5484d").ok(),
        "{:?}",
        seen.borrow()
    );
    assert_eq!(
        page.file_json()["clusters"][prod.as_str()]["colour"],
        json!("#e5484d")
    );
    // Another cluster still has none.
    let other = page
        .vcx
        .update(|_, cx| ClusterSettings::resolve(&dev, cx).colour);
    assert_eq!(other, None);
}

#[gpui::test]
fn the_list_comes_from_the_catalog_and_names_clusters_by_display_name(cx: &mut TestAppContext) {
    let prod = id("prod");
    let user =
        format!("{{ \"clusters\": {{ \"{prod}\": {{ \"display_name\": \"Production\" }} }} }}");
    let mut page = Harness::open(cx, &user, &["prod", "dev"]);
    assert!(page.is_laid_out(&format!("settings-cluster-{prod}")));
    assert!(page.is_laid_out(&format!("settings-cluster-{}", id("dev"))));
    let labels: Vec<String> = page.vcx.read(|cx| {
        let store = cx.try_global::<SettingsStore>();
        page.page
            .read(cx)
            .cluster_choices()
            .iter()
            .map(|row| row.label(store).to_string())
            .collect()
    });
    assert_eq!(labels, ["Production", "dev"]);
}

#[gpui::test]
fn the_page_lists_the_clusters_of_a_real_catalog(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    let config = tempfile::tempdir().expect("a temporary config directory");
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([entry("prod").context, entry("dev").context]),
    );
    let catalog = ClusterCatalog::new(
        source,
        Arc::new(FakeStatePort::new()),
        Arc::new(FakeClockPort::default()),
    );
    let mut page = None;
    vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        crate::page::test_support::install_settings(config.path(), "{}", cx);
        let entity = cx.new(|cx| {
            let mut page =
                SettingsPage::with_model(crate::page::test_support::fixture_model(), window, cx);
            page.set_catalog(catalog, cx);
            page
        });
        page = Some(entity.clone());
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(
                Box::new(entity),
                oxikube_workspace::OpenOptions::default(),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();
    let page = page.expect("the page was built");
    let names = page.read_with(&vcx, |page, _| {
        page.cluster_choices()
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(names.len(), 2);
    assert!(names.contains(&id("prod")) && names.contains(&id("dev")));
}

#[gpui::test]
fn the_same_field_typed_for_two_clusters_inside_one_pause_reaches_both(cx: &mut TestAppContext) {
    let (prod, dev) = (id("prod"), id("dev"));
    let mut page = Harness::open(cx, "{}", &["prod", "dev"]);
    page.type_into("display_name", "Production");
    page.click(&format!("settings-cluster-{}", id("dev")));
    page.type_into("display_name", "Development");
    page.pause(WRITE_DEBOUNCE * 2);
    let file = page.file_json();
    assert_eq!(
        file["clusters"][prod.as_str()]["display_name"],
        json!("Production"),
        "{file}"
    );
    assert_eq!(
        file["clusters"][dev.as_str()]["display_name"],
        json!("Development"),
        "{file}"
    );
}

#[gpui::test]
fn a_value_queued_behind_another_clusters_write_keeps_its_own_cluster(cx: &mut TestAppContext) {
    let (prod, dev) = (id("prod"), id("dev"));
    let mut page = Harness::open(cx, "{}", &["prod", "dev"]);
    page.type_into("display_name", "Production");
    // Let the first timer fire (write goes in flight) without letting the write finish.
    page.pause(WRITE_DEBOUNCE);
    page.click(&format!("settings-cluster-{}", id("dev")));
    page.type_into("display_name", "Development");
    page.pause(WRITE_DEBOUNCE * 2);
    let file = page.file_json();
    assert_eq!(
        file["clusters"][prod.as_str()]["display_name"],
        json!("Production"),
        "{file}"
    );
    assert_eq!(
        file["clusters"][dev.as_str()]["display_name"],
        json!("Development"),
        "{file}"
    );
}
