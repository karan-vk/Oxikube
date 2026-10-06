use std::path::{Path, PathBuf};

use oxikube_ports::UserSource;

use super::*;
use crate::store::SettingsStore;
use crate::update::new_text_for_update;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<KubeconfigSettings>();
    store.set_user_settings(user).unwrap();
    store
}

fn entry(kind: KubeconfigSourceKind, path: Option<&str>) -> KubeconfigSourceEntry {
    KubeconfigSourceEntry {
        kind,
        path: path.map(Into::into),
    }
}

#[test]
fn the_default_list_reads_kubectls_files_and_matches_default_json() {
    let store = store("{}");
    let settings = store.get::<KubeconfigSettings>(None);
    assert_eq!(settings.sources, default_sources());
    assert_eq!(settings.user_sources(), vec![UserSource::default_source()]);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn the_user_list_replaces_the_default_as_a_whole() {
    let store = store(
        r#"{ "kubeconfig": { "sources": [
            { "kind": "file", "path": "/work/prod.yaml" },
            { "kind": "dir", "path": "/work/configs" }
        ] } }"#,
    );
    let settings = store.get::<KubeconfigSettings>(None);
    assert_eq!(
        settings.user_sources(),
        vec![
            UserSource::file("/work/prod.yaml"),
            UserSource::dir("/work/configs")
        ],
        "no default entry: kubectl's files are not read"
    );
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn an_empty_list_is_allowed() {
    let store = store(r#"{ "kubeconfig": { "sources": [] } }"#);
    assert!(
        store
            .get::<KubeconfigSettings>(None)
            .user_sources()
            .is_empty()
    );
}

#[test]
fn entries_without_a_path_and_repeats_are_skipped() {
    let settings = KubeconfigSettings {
        sources: vec![
            entry(KubeconfigSourceKind::File, None),
            entry(KubeconfigSourceKind::Dir, Some("  ")),
            entry(KubeconfigSourceKind::File, Some("/a.yaml")),
            entry(KubeconfigSourceKind::File, Some("/a.yaml")),
            entry(KubeconfigSourceKind::Default, Some("ignored")),
            entry(KubeconfigSourceKind::Default, None),
        ],
    };
    assert_eq!(
        settings.user_sources(),
        vec![UserSource::file("/a.yaml"), UserSource::default_source()]
    );
}

#[test]
fn a_leading_tilde_is_the_home_directory() {
    let home = Path::new("/home/me");
    let to = |kind, path: &str| {
        entry(kind, Some(path))
            .to_user_source(Some(home))
            .and_then(|s| s.path)
    };
    assert_eq!(
        to(KubeconfigSourceKind::Dir, "~/clusters"),
        Some(PathBuf::from("/home/me/clusters"))
    );
    assert_eq!(
        to(KubeconfigSourceKind::File, "~"),
        Some(PathBuf::from("/home/me"))
    );
    assert_eq!(
        to(KubeconfigSourceKind::File, "~other/x"),
        Some(PathBuf::from("~other/x")),
        "only the current user's home"
    );
    assert_eq!(
        entry(KubeconfigSourceKind::File, Some("~/x"))
            .to_user_source(None)
            .and_then(|s| s.path),
        Some(PathBuf::from("~/x")),
        "without a home directory the path is left alone"
    );
}

#[test]
fn an_unknown_kind_is_reported_and_the_previous_list_stays() {
    let mut store =
        store(r#"{ "kubeconfig": { "sources": [ { "kind": "file", "path": "/a" } ] } }"#);
    let result =
        store.set_user_settings(r#"{ "kubeconfig": { "sources": [ { "kind": "ftp" } ] } }"#);
    assert!(result.is_ok());
    assert!(!store.diagnostics().is_empty());
    assert_eq!(
        store.get::<KubeconfigSettings>(None).user_sources(),
        vec![UserSource::file("/a")]
    );
}

#[test]
fn an_edit_writes_the_list_and_keeps_the_users_comments() {
    let old = "{\n  // my theme\n  \"ui_scale\": 1.25\n}\n";
    let sources = vec![
        KubeconfigSourceEntry::from_user_source(&UserSource::default_source()),
        KubeconfigSourceEntry::from_user_source(&UserSource::file("/config/kubeconfigs/prod.yaml")),
    ];
    let text = new_text_for_update::<KubeconfigSettings>(old, None, move |content| {
        content.sources = Some(sources);
    })
    .unwrap();
    assert!(text.contains("// my theme"), "{text}");
    assert!(text.contains("\"ui_scale\": 1.25"), "{text}");

    let reloaded = store(&text);
    assert_eq!(
        reloaded.get::<KubeconfigSettings>(None).user_sources(),
        vec![
            UserSource::default_source(),
            UserSource::file("/config/kubeconfigs/prod.yaml")
        ]
    );
}

#[test]
fn the_schema_describes_the_key() {
    let store = store("{}");
    let schema = serde_json::to_string(&store.json_schema()).unwrap();
    assert!(schema.contains("\"kubeconfig\""), "{schema}");
    assert!(schema.contains("sources"));
}
