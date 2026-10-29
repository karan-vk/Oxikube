use std::path::PathBuf;

use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<KustomizeSettings>();
    store.set_user_settings(user).unwrap();
    store
}

#[test]
fn the_default_autodetects() {
    let store = store("{}");
    assert_eq!(store.get::<KustomizeSettings>(None).path, None);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn a_user_path_is_used_and_a_blank_one_autodetects() {
    let store = store(r#"{ "kustomize": { "path": "/opt/k/kustomize" } }"#);
    assert_eq!(
        store.get::<KustomizeSettings>(None).path,
        Some(PathBuf::from("/opt/k/kustomize"))
    );
    let blank = self::store(r#"{ "kustomize": { "path": "  " } }"#);
    assert_eq!(blank.get::<KustomizeSettings>(None).path, None);
}
