//! GPUI-level tests of the `metrics` setting (E13-S02): the table handed to the app layer, and
//! hot reload through the observer. `init_with_dir` only (no watcher thread); the reload path is
//! `SettingsStore::set_user_settings`, what the watcher's apply task calls.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::utilisation::GpuVendors;
use oxikube_settings::{MetricsSettings, Settings as _, SettingsStore, init_with_dir};

const PROD: &str = "3f2a9c1b7d4e8a60";
const LAB: &str = "0011223344556677";

fn id(text: &str) -> ClusterId {
    text.parse().expect("test cluster id is 16 hex digits")
}

fn setup(cx: &mut TestAppContext, user: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temp config dir");
    std::fs::write(dir.path().join("settings.json"), user).expect("write settings.json fixture");
    cx.update(|cx| init_with_dir(dir.path(), cx));
    dir
}

fn reload(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(&text)
                .expect("test settings text parses")
        })
    });
    cx.run_until_parked();
}

#[gpui::test]
fn the_table_has_the_defaults_and_the_clusters_overrides(cx: &mut TestAppContext) {
    let _dir = setup(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{PROD}": {{ "metrics": {{ "gpu_vendors": ["x/gpu"] }} }} }} }}"#
        ),
    );
    cx.read(|cx| {
        let table = MetricsSettings::gpu_table(cx);
        assert_eq!(table.default_vendors(), &GpuVendors::default());
        assert!(table.for_cluster(&id(PROD)).contains("x/gpu"));
        assert_eq!(table.for_cluster(&id(LAB)), &GpuVendors::default());
    });
}

#[gpui::test]
fn an_edit_reaches_the_observer_and_unrelated_edits_do_not(cx: &mut TestAppContext) {
    let _dir = setup(cx, "{}");
    let calls = Rc::new(Cell::new(0));
    let sink = calls.clone();
    let _sub = cx.update(|cx| MetricsSettings::observe(cx, move |_| sink.set(sink.get() + 1)));

    reload(cx, r#"{ "ui_scale": 1.5 }"#);
    assert_eq!(calls.get(), 0, "an unrelated setting changed");

    reload(
        cx,
        r#"{ "metrics": { "gpu_vendors": ["a/gpu", "b/gpu"] } }"#,
    );
    assert_eq!(calls.get(), 1);
    cx.read(|cx| {
        let table = MetricsSettings::gpu_table(cx);
        assert!(table.default_vendors().contains("b/gpu"));
        assert!(!table.default_vendors().contains("nvidia.com/gpu"));
    });

    // A cluster's block changes: the observer hears it (it covers every cluster).
    reload(
        cx,
        &format!(
            r#"{{ "metrics": {{ "gpu_vendors": ["a/gpu", "b/gpu"] }},
                "clusters": {{ "{LAB}": {{ "metrics": {{ "gpu_vendors": [] }} }} }} }}"#
        ),
    );
    assert_eq!(calls.get(), 2);
    cx.read(|cx| {
        assert!(
            MetricsSettings::gpu_table(cx)
                .for_cluster(&id(LAB))
                .is_empty()
        )
    });
}
