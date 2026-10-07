//! The per-cluster layer: precedence per field, diagnostics, comment-preserving edits, schema.

use oxikube_domain::ClusterColour;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{ClusterPrefs, ExecInteractivity};
use serde_json::{Value, json};

use super::{ClusterSettings, ClusterSettingsContent};
use crate::diagnostics::SettingsDiagnostic;
use crate::settings::SettingsLocation;
use crate::store::SettingsStore;
use crate::update::new_text_for_update;

const PROD: &str = "3f2a9c1b7d4e8a60";
const LAB: &str = "0011223344556677";

fn id(text: &str) -> ClusterId {
    text.parse().unwrap()
}

/// A store over the shipped `default.json` with the cluster settings registered.
fn store() -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<ClusterSettings>();
    store
}

fn load(user: &str) -> SettingsStore {
    let mut store = store();
    store.set_user_settings(user).unwrap();
    store
}

fn prefs<'a>(store: &'a SettingsStore, cluster: Option<&ClusterId>) -> &'a ClusterPrefs {
    store
        .get::<ClusterSettings>(cluster.map(|cluster| SettingsLocation { cluster }))
        .prefs()
}

#[test]
fn defaults_leave_everything_unset_and_writable() {
    let store = store();
    assert_eq!(prefs(&store, None), &ClusterPrefs::default());
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}

/// For each field: the default, then the user's top-level value, then the cluster's own value.
/// `lab` has no block and always reads the user layer; `prod` reads its own block over it.
#[test]
fn the_cluster_layer_beats_the_user_layer_beats_the_defaults_for_every_field() {
    // (key, user value, cluster value, how to read it back)
    type Read = fn(&ClusterPrefs) -> Value;
    let cases: Vec<(&str, Value, Value, Read)> = vec![
        ("display_name", json!("User"), json!("Cluster"), |p| {
            json!(p.display_name)
        }),
        ("colour", json!("#111111"), json!("#222222"), |p| {
            json!(p.colour.map(|c| c.to_string()))
        }),
        ("read_only", json!(true), json!(false), |p| {
            json!(p.read_only)
        }),
        (
            "default_namespace",
            json!("user-ns"),
            json!("cluster-ns"),
            |p| json!(p.default_namespace),
        ),
        ("terminal_cwd", json!("/user"), json!("/cluster"), |p| {
            json!(p.terminal_cwd)
        }),
        (
            "node_shell_image",
            json!("user:1"),
            json!("cluster:1"),
            |p| json!(p.node_shell_image),
        ),
        (
            "node_shell_pull_secret",
            json!("user-s"),
            json!("cluster-s"),
            |p| json!(p.node_shell_pull_secret),
        ),
        (
            "accessible_namespaces",
            json!(["u1", "u2"]),
            json!(["c1"]),
            |p| json!(p.accessible_namespaces),
        ),
        (
            "exec_interactivity",
            json!("always"),
            json!("if_available"),
            |p| json!(p.exec_interactivity),
        ),
        ("exec_in_read_only", json!(true), json!(false), |p| {
            json!(p.exec_in_read_only)
        }),
        (
            "prometheus",
            json!({"provider": "auto"}),
            json!({"provider": "operator"}),
            |p| json!({ "provider": p.prometheus.as_ref().and_then(|v| v.provider.clone()) }),
        ),
    ];
    for (key, user, cluster, read) in cases {
        let default = read(&ClusterPrefs::default());
        let text = format!(
            "{{ {key:?}: {user}, \"clusters\": {{ {PROD:?}: {{ {key:?}: {cluster} }} }} }}"
        );
        let store = load(&text);
        assert!(
            store.diagnostics().is_empty(),
            "{key}: {:?}",
            store.diagnostics()
        );
        assert_eq!(read(prefs(&store, None)), user, "{key}: user over defaults");
        assert_eq!(
            read(prefs(&store, Some(&id(LAB)))),
            user,
            "{key}: no block reads user"
        );
        assert_eq!(
            read(prefs(&store, Some(&id(PROD)))),
            cluster,
            "{key}: cluster over user"
        );
        assert_ne!(default, user, "{key}: the case must change the default");
    }
}

#[test]
fn a_cluster_block_overrides_only_the_fields_it_names() {
    let store = load(&format!(
        r##"{{
          "read_only": true,
          "terminal_cwd": "/work",
          "clusters": {{ "{PROD}": {{ "colour": "#e5484d", "default_namespace": "payments" }} }}
        }}"##
    ));
    let prod = prefs(&store, Some(&id(PROD)));
    assert!(prod.read_only, "inherited from the top level");
    assert_eq!(prod.terminal_cwd.as_deref(), Some("/work"));
    assert_eq!(prod.colour, Some(ClusterColour::rgb(0xe5, 0x48, 0x4d)));
    assert_eq!(prod.default_namespace.as_deref(), Some("payments"));
}

#[test]
fn prometheus_fields_merge_and_hold_only_a_keychain_reference() {
    let store = load(&format!(
        r#"{{
          "prometheus": {{ "provider": "auto" }},
          "clusters": {{ "{PROD}": {{ "prometheus": {{
            "url": "https://prom.example.com", "auth_secret": "prod-prom" }} }} }}
        }}"#
    ));
    let prom = prefs(&store, Some(&id(PROD))).prometheus.clone().unwrap();
    assert_eq!(prom.provider.as_deref(), Some("auto"));
    assert_eq!(prom.url.as_deref(), Some("https://prom.example.com"));
    assert_eq!(prom.auth.unwrap().to_string(), "prometheus/prod-prom");
}

#[test]
fn blank_strings_are_unset_and_namespace_lists_are_cleaned() {
    let store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{
          "display_name": "  ", "default_namespace": "",
          "accessible_namespaces": [" a ", "b", "a", ""]
        }} }} }}"#
    ));
    let prod = prefs(&store, Some(&id(PROD)));
    assert_eq!(prod.display_name, None);
    assert_eq!(prod.default_namespace, None);
    assert_eq!(prod.accessible_namespaces, ["a", "b"]);
}

#[test]
fn unknown_keys_in_a_cluster_block_are_reported_with_their_path() {
    let store = load(&format!(
        r##"{{ "clusters": {{ "{PROD}": {{ "read_onyl": true, "colour": "#fff" }} }} }}"##
    ));
    assert_eq!(
        store.diagnostics(),
        [SettingsDiagnostic::UnknownKey {
            path: format!("clusters.{PROD}.read_onyl")
        }]
    );
    // The known key still applies.
    assert!(prefs(&store, Some(&id(PROD))).colour.is_some());
}

#[test]
fn a_type_error_keeps_the_previous_value_and_names_the_field() {
    let mut store = load(&format!(
        r##"{{ "clusters": {{ "{PROD}": {{ "read_only": true, "colour": "#e5484d" }} }} }}"##
    ));
    let good = prefs(&store, Some(&id(PROD))).clone();
    let generation = store.generation::<ClusterSettings>();

    store
        .set_user_settings(&format!(
            r#"{{ "clusters": {{ "{PROD}": {{ "read_only": false, "colour": "red" }} }} }}"#
        ))
        .unwrap();

    assert_eq!(
        prefs(&store, Some(&id(PROD))),
        &good,
        "the last good block stays"
    );
    assert_eq!(
        store.generation::<ClusterSettings>(),
        generation,
        "nothing changed"
    );
    let [
        SettingsDiagnostic::InvalidValue {
            cluster, message, ..
        },
    ] = store.diagnostics()
    else {
        panic!("{:?}", store.diagnostics());
    };
    assert_eq!(cluster.as_deref(), Some(PROD));
    assert!(message.contains("colour"), "{message}");
}

#[test]
fn a_typo_next_to_read_only_never_leaves_the_cluster_writable_on_a_first_load() {
    let store = load(&format!(
        r##"{{ "clusters": {{ "{PROD}": {{ "read_only": true, "colour": "red" }} }} }}"##
    ));
    assert!(prefs(&store, Some(&id(PROD))).read_only);
    assert!(
        !prefs(&store, Some(&id(LAB))).read_only,
        "others unaffected"
    );
    assert_eq!(store.diagnostics().len(), 1);
}

#[test]
fn a_typo_in_a_top_level_key_keeps_a_top_level_read_only_on_a_first_load() {
    let store = load(&format!(
        r#"{{ "read_only": true, "exec_interactivity": "bogus",
              "clusters": {{ "{PROD}": {{ "default_namespace": "pay" }} }} }}"#
    ));
    assert!(prefs(&store, None).read_only, "global");
    assert!(
        prefs(&store, Some(&id(LAB))).read_only,
        "a cluster without a block"
    );
    // The cluster's own block sees the same bad key, and still reads as read-only.
    assert!(
        prefs(&store, Some(&id(PROD))).read_only,
        "a cluster with a block"
    );
}

#[test]
fn a_typo_next_to_a_new_read_only_true_takes_effect_on_a_reload() {
    let mut store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{ "default_namespace": "pay" }} }} }}"#
    ));
    assert!(!prefs(&store, Some(&id(PROD))).read_only);

    store
        .set_user_settings(&format!(
            r##"{{ "clusters": {{ "{PROD}": {{ "read_only": true, "colour": "red" }} }} }}"##
        ))
        .unwrap();

    let now = prefs(&store, Some(&id(PROD)));
    assert!(now.read_only, "protection is added");
    assert_eq!(
        now.default_namespace.as_deref(),
        Some("pay"),
        "rest stays last good"
    );
}

#[test]
fn a_read_only_that_is_not_a_bool_fails_closed() {
    let store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{ "read_only": "yes" }} }} }}"#
    ));
    assert!(prefs(&store, Some(&id(PROD))).read_only);
    assert_eq!(store.diagnostics().len(), 1, "still reported");
}

#[test]
fn an_explicit_false_with_a_typo_does_not_lift_a_global_read_only() {
    let store = load(&format!(
        r##"{{ "read_only": true,
              "clusters": {{ "{PROD}": {{ "read_only": false, "colour": "red" }} }} }}"##
    ));
    assert!(
        prefs(&store, Some(&id(PROD))).read_only,
        "the bad block is not applied"
    );
}

#[test]
fn a_type_error_in_the_wrong_kind_of_value_is_helpful() {
    for (field, bad, needle) in [
        ("read_only", json!("yes"), "read_only"),
        (
            "exec_interactivity",
            json!("sometimes"),
            "exec_interactivity",
        ),
        (
            "accessible_namespaces",
            json!("kube-system"),
            "accessible_namespaces",
        ),
        (
            "prometheus",
            json!({"url": "https://u:p@prom"}),
            "prometheus.url",
        ),
        (
            "prometheus",
            json!({"auth_secret": "a/b"}),
            "prometheus.auth_secret",
        ),
    ] {
        let store = load(&format!(
            r#"{{ "clusters": {{ "{PROD}": {{ {field:?}: {bad} }} }} }}"#
        ));
        let [SettingsDiagnostic::InvalidValue { message, .. }] = store.diagnostics() else {
            panic!("{field}: {:?}", store.diagnostics());
        };
        assert!(message.contains(needle), "{message}");
        assert!(
            !message.contains("p@prom"),
            "must not echo the URL: {message}"
        );
    }
}

#[test]
fn a_new_value_for_one_cluster_wakes_only_that_clusters_value() {
    let mut store = load(&format!(
        r#"{{ "clusters": {{ "{PROD}": {{ "read_only": true }}, "{LAB}": {{ "terminal_cwd": "/lab" }} }} }}"#
    ));
    let before_lab = prefs(&store, Some(&id(LAB))).clone();
    store
        .set_user_settings(&format!(
            r##"{{ "clusters": {{ "{PROD}": {{ "read_only": true, "colour": "#fff" }}, "{LAB}": {{ "terminal_cwd": "/lab" }} }} }}"##
        ))
        .unwrap();
    assert_eq!(prefs(&store, Some(&id(LAB))), &before_lab);
    assert!(prefs(&store, Some(&id(PROD))).colour.is_some());
}

const USER_FILE: &str = r##"// my settings
{
  "ui_scale": 1.25, // bigger
  "clusters": {
    // production
    "3f2a9c1b7d4e8a60": {
      "colour": "#e5484d", // red
      "read_only": false,
    },
  },
}
"##;

#[test]
fn toggling_read_only_rewrites_one_value_and_keeps_the_comments() {
    let text = new_text_for_update::<ClusterSettings>(USER_FILE, Some(&id(PROD)), |content| {
        content.read_only = Some(true);
    })
    .unwrap();
    assert_eq!(
        text,
        USER_FILE.replace("\"read_only\": false", "\"read_only\": true")
    );
}

#[test]
fn a_new_cluster_block_is_added_and_the_rest_of_the_file_is_untouched() {
    let text = new_text_for_update::<ClusterSettings>(USER_FILE, Some(&id(LAB)), |content| {
        content.read_only = Some(true);
    })
    .unwrap();
    assert!(text.starts_with("// my settings\n{\n  \"ui_scale\": 1.25, // bigger\n"));
    assert!(text.contains("// production") && text.contains("// red"));
    let parsed = crate::jsonc::parse_jsonc_object(&text).unwrap();
    assert_eq!(parsed["clusters"][LAB], json!({"read_only": true}));
    assert_eq!(parsed["clusters"][PROD]["colour"], json!("#e5484d"));
}

#[test]
fn clearing_a_field_removes_it_from_the_block() {
    let text = new_text_for_update::<ClusterSettings>(USER_FILE, Some(&id(PROD)), |content| {
        content.colour = None;
    })
    .unwrap();
    assert!(!text.contains("#e5484d"));
    assert!(text.contains("\"read_only\": false"));
}

#[test]
fn content_round_trips_without_null_noise() {
    let content = ClusterSettingsContent {
        read_only: Some(true),
        exec_interactivity: Some(ExecInteractivity::IfAvailable),
        ..ClusterSettingsContent::default()
    };
    let text = serde_json::to_string(&content).unwrap();
    assert_eq!(
        text,
        r#"{"read_only":true,"exec_interactivity":"if_available"}"#
    );
    assert_eq!(
        serde_json::from_str::<ClusterSettingsContent>(&text).unwrap(),
        content
    );
}

#[test]
fn the_schema_documents_the_cluster_block_with_an_example() {
    let schema = store().json_schema();
    let cluster = &schema["$defs"]["ClusterSettings"];
    let keys = [
        "display_name",
        "colour",
        "read_only",
        "default_namespace",
        "terminal_cwd",
        "node_shell_image",
        "node_shell_pull_secret",
        "prometheus",
        "accessible_namespaces",
        "exec_interactivity",
        "exec_in_read_only",
    ];
    for key in keys {
        let property = &cluster["properties"][key];
        assert!(
            property.is_object(),
            "{key} is missing from the cluster block"
        );
        assert!(
            property["description"].is_string()
                || schema["$defs"]["ClusterSettingsContent"]["properties"][key]["description"]
                    .is_string(),
            "{key} has no description"
        );
    }
    assert_eq!(
        cluster["additionalProperties"],
        json!(false),
        "typos are flagged by editors"
    );
    let example = &schema["properties"]["clusters"]["examples"][0];
    assert!(
        example.is_object(),
        "the clusters key carries a documented example"
    );
    assert_eq!(
        schema["properties"]["clusters"]["additionalProperties"]["$ref"],
        json!("#/$defs/ClusterSettings")
    );
}

#[test]
fn the_documented_example_validates_against_the_content_type() {
    let schema = store().json_schema();
    let example = &schema["properties"]["clusters"]["examples"][0];
    let text = serde_json::to_string(&json!({ "clusters": example })).unwrap();
    let store = load(&text);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
}
