//! GPUI-level tests of the per-cluster layer: observers scoped to one cluster, in-app edits
//! that keep comments, and the lookup table handed to the app layer. `init_with_dir` only (no
//! watcher thread); the reload path is `SettingsStore::set_user_settings`, what the watcher's
//! apply task calls.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Subscription, TestAppContext, UpdateGlobal as _};
use oxikube_domain::ClusterColour;
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{ClusterSettings, SettingsStore, init_with_dir};

const PROD: &str = "3f2a9c1b7d4e8a60";
const LAB: &str = "0011223344556677";
const USER: &str = "// mine\n{\n  \"ui_scale\": 1.5, // keep me\n}\n";

fn id(text: &str) -> ClusterId {
    text.parse().unwrap()
}

fn setup(cx: &mut TestAppContext, user: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.json"), user).unwrap();
    cx.update(|cx| init_with_dir(dir.path(), cx));
    dir
}

fn reload(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| store.set_user_settings(&text).unwrap())
    });
    cx.run_until_parked();
}

type Seen = Rc<RefCell<Vec<ClusterSettings>>>;

fn watch(cx: &mut TestAppContext, cluster: &str) -> (Seen, Subscription) {
    let seen: Seen = Rc::default();
    let sink = seen.clone();
    let sub = cx.update(|cx| {
        ClusterSettings::observe_cluster(cx, id(cluster), move |settings, _| {
            sink.borrow_mut().push(settings.clone());
        })
    });
    (seen, sub)
}

#[gpui::test]
fn the_store_registers_cluster_settings_with_shipped_defaults(cx: &mut TestAppContext) {
    let _dir = setup(cx, USER);
    cx.read(|cx| {
        let settings = ClusterSettings::resolve(&id(PROD), cx);
        assert!(!settings.read_only);
        assert_eq!(settings.colour, None);
        // Other crates' defaults are unknown keys here (they are not linked); ours are fine.
        let diagnostics = cx.global::<SettingsStore>().diagnostics();
        assert!(
            diagnostics
                .iter()
                .all(|d| !d.to_string().contains("ClusterSettings")),
            "{diagnostics:?}"
        );
        assert!(ClusterSettings::table(cx).is_empty());
    });
}

#[gpui::test]
fn a_cluster_observer_ignores_other_clusters_and_unrelated_edits(cx: &mut TestAppContext) {
    let _dir = setup(cx, USER);
    let (prod, _prod_sub) = watch(cx, PROD);
    let (lab, _lab_sub) = watch(cx, LAB);

    reload(
        cx,
        &format!(r#"{{ "clusters": {{ "{PROD}": {{ "read_only": true }} }} }}"#),
    );
    assert_eq!((prod.borrow().len(), lab.borrow().len()), (1, 0));
    assert!(prod.borrow()[0].read_only);

    // An unrelated setting changes: nobody is told.
    reload(
        cx,
        &format!(r#"{{ "ui_scale": 2, "clusters": {{ "{PROD}": {{ "read_only": true }} }} }}"#),
    );
    assert_eq!((prod.borrow().len(), lab.borrow().len()), (1, 0));

    // Another cluster's block changes: only that cluster's observer runs.
    reload(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{PROD}": {{ "read_only": true }}, "{LAB}": {{ "terminal_cwd": "/lab" }} }} }}"#
        ),
    );
    assert_eq!((prod.borrow().len(), lab.borrow().len()), (1, 1));

    // A top-level change that every cluster inherits wakes the ones whose value changed.
    reload(
        cx,
        &format!(
            r#"{{ "terminal_cwd": "/work", "clusters": {{ "{PROD}": {{ "read_only": true }}, "{LAB}": {{ "terminal_cwd": "/lab" }} }} }}"#
        ),
    );
    assert_eq!((prod.borrow().len(), lab.borrow().len()), (2, 1));
    assert_eq!(prod.borrow()[1].terminal_cwd.as_deref(), Some("/work"));
}

#[gpui::test]
fn a_bad_edit_keeps_the_value_and_wakes_nobody(cx: &mut TestAppContext) {
    let _dir = setup(cx, USER);
    reload(
        cx,
        &format!(r#"{{ "clusters": {{ "{PROD}": {{ "read_only": true }} }} }}"#),
    );
    let (prod, _sub) = watch(cx, PROD);

    reload(
        cx,
        &format!(r#"{{ "clusters": {{ "{PROD}": {{ "read_only": "yes" }} }} }}"#),
    );

    assert!(prod.borrow().is_empty());
    cx.read(|cx| {
        assert!(ClusterSettings::resolve(&id(PROD), cx).read_only);
        assert_eq!(cx.global::<SettingsStore>().diagnostics().len(), 1);
    });
}

#[gpui::test]
async fn the_read_only_toggle_edits_the_file_and_keeps_the_users_comments(cx: &mut TestAppContext) {
    let dir = setup(cx, USER);
    let (prod, _sub) = watch(cx, PROD);

    cx.update(|cx| ClusterSettings::set_read_only(cx, &id(PROD), Some("prod-eu"), true))
        .await
        .unwrap();
    cx.run_until_parked();

    cx.read(|cx| assert!(ClusterSettings::resolve(&id(PROD), cx).read_only));
    assert_eq!(prod.borrow().len(), 1, "one change, one notification");
    let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
    assert!(text.starts_with("// mine\n"), "{text}");
    assert!(text.contains("\"ui_scale\": 1.5, // keep me"), "{text}");
    // The new entry is recognisable next to its opaque id.
    assert!(text.contains(&format!("\"{PROD}\"")), "{text}");
    assert!(text.contains("\"display_name\": \"prod-eu\""), "{text}");

    // Toggling again rewrites one value and does not rename a block that exists.
    cx.update(|cx| ClusterSettings::set_read_only(cx, &id(PROD), Some("other name"), false))
        .await
        .unwrap();
    let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
    assert!(text.contains("\"read_only\": false"), "{text}");
    assert!(text.contains("\"display_name\": \"prod-eu\""), "{text}");
    assert!(!text.contains("other name"), "{text}");
}

#[gpui::test]
fn the_table_indexes_every_cluster_with_overrides(cx: &mut TestAppContext) {
    let _dir = setup(cx, USER);
    reload(
        cx,
        &format!(
            r##"{{
              "read_only": true,
              "clusters": {{
                "{PROD}": {{ "colour": "#e5484d", "read_only": false }},
                "not-an-id": {{ "colour": "#000" }}
              }}
            }}"##
        ),
    );
    cx.read(|cx| {
        let table = ClusterSettings::table(cx);
        assert_eq!(table.len(), 1, "the malformed key is skipped");
        let prod = table.get(&id(PROD));
        assert!(!prod.read_only);
        assert_eq!(prod.colour, Some(ClusterColour::rgb(0xe5, 0x48, 0x4d)));
        assert!(
            table.get(&id(LAB)).read_only,
            "other clusters read the top level"
        );
    });
}
