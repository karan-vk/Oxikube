use std::path::PathBuf;

use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<HelmSettings>();
    store.set_user_settings(user).unwrap();
    store
}

fn names(settings: &HelmSettings) -> Vec<&str> {
    settings
        .default_repos
        .iter()
        .map(|r| r.name.as_str())
        .collect()
}

#[test]
fn the_defaults_autodetect_skip_server_side_and_offer_a_small_repo_set() {
    let store = store("{}");
    let settings = store.get::<HelmSettings>(None);
    assert_eq!(settings.path, None, "empty path means autodetect");
    assert!(!settings.server_side_apply);
    assert_eq!(
        names(settings),
        ["bitnami", "ingress-nginx", "prometheus-community"]
    );
    assert!(
        settings
            .default_repos
            .iter()
            .all(|r| r.url.starts_with("https://"))
    );
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

#[test]
fn a_user_layer_sets_each_key_and_replaces_the_repo_list() {
    let store = store(
        r#"{ "helm": { "path": "/opt/helm/bin/helm", "server_side_apply": true,
             "default_repos": [{ "name": "mine", "url": "https://charts.example.com" }] } }"#,
    );
    let s = store.get::<HelmSettings>(None);
    assert_eq!(s.path, Some(PathBuf::from("/opt/helm/bin/helm")));
    assert!(s.server_side_apply);
    assert_eq!(names(s), ["mine"]);
}

#[test]
fn a_blank_path_is_autodetect() {
    let store = store(r#"{ "helm": { "path": "   " } }"#);
    assert_eq!(store.get::<HelmSettings>(None).path, None);
}

#[test]
fn unusable_repos_are_dropped_and_the_rest_kept() {
    let store = store(
        r#"{ "helm": { "default_repos": [
            { "name": "ok", "url": "https://a.example.com/charts" },
            { "name": "creds", "url": "https://user:pw@b.example.com" },
            { "name": "ftp", "url": "ftp://c.example.com" },
            { "name": "oci", "url": "oci://registry.example.com/charts" },
            { "name": "", "url": "https://d.example.com" },
            { "name": "has space", "url": "https://e.example.com" },
            { "name": "nohost", "url": "https:///charts" },
            { "name": "ok", "url": "https://dup.example.com" },
            { "name": "second", "url": "http://f.example.com" }
        ] } }"#,
    );
    assert_eq!(names(store.get::<HelmSettings>(None)), ["ok", "second"]);
}

#[test]
fn a_wrong_type_is_reported_and_the_previous_value_stays() {
    let mut store = store(r#"{ "helm": { "path": "/a/helm" } }"#);
    store
        .set_user_settings(r#"{ "helm": { "path": 7 } }"#)
        .unwrap();
    assert!(!store.diagnostics().is_empty());
    assert_eq!(
        store.get::<HelmSettings>(None).path,
        Some(PathBuf::from("/a/helm"))
    );
}

#[test]
fn the_schema_describes_the_keys() {
    let schema = serde_json::to_string(&store("{}").json_schema()).unwrap();
    for key in [
        "helm",
        "server_side_apply",
        "default_repos",
        "HelmRepoContent",
    ] {
        assert!(schema.contains(key), "{key} missing from {schema}");
    }
}

#[test]
fn server_side_warns_only_on_an_old_binary_with_the_flag_on() {
    let on = HelmSettings {
        server_side_apply: true,
        ..HelmSettings::default()
    };
    let off = HelmSettings::default();
    let old = HelmCapabilities::full(HelmVersion::new(3, 12, 3));
    let new = HelmCapabilities::full(HelmVersion::new(3, 13, 0));
    let warning = server_side_warning(&on, &old).expect("3.12 cannot do server-side");
    assert!(
        warning.contains("3.12.3") && warning.contains("3.13.0"),
        "{warning}"
    );
    assert_eq!(server_side_warning(&on, &new), None);
    assert_eq!(server_side_warning(&off, &old), None);
    assert_eq!(
        server_side_warning(&on, &HelmCapabilities::native_only()),
        None
    );
}

/// Hot reload: an edit of `helm.path` wakes `observe`rs; an unrelated edit does not.
#[gpui::test]
fn editing_the_path_wakes_observers_and_other_edits_do_not(cx: &mut gpui::TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::UpdateGlobal as _;

    cx.update(|cx| {
        let mut store =
            SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
        store.register_setting::<HelmSettings>();
        cx.set_global(store);
    });
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        HelmSettings::observe(cx, move |cx| {
            sink.borrow_mut().push(HelmSettings::resolve(cx).path);
        })
        .detach();
    });
    let set = |cx: &mut gpui::TestAppContext, text: &'static str| {
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, _| store.set_user_settings(text).unwrap());
        });
    };
    set(cx, r#"{ "helm": { "path": "/x/helm" } }"#);
    set(
        cx,
        r#"{ "helm": { "path": "/x/helm" }, "portforward": {} }"#,
    );
    set(cx, "{}");
    assert_eq!(
        *seen.borrow(),
        [Some(PathBuf::from("/x/helm")), None],
        "one wake per real change of the helm value"
    );
}
