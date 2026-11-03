//! The per-cluster model and the cluster list as plain data.

use oxikube_settings::{CLUSTER_SCOPED_KEY, SettingsStore};
use serde_json::json;

use super::{entry, id};
use crate::page::cluster_tab::{choices, cluster_page_model, supports_cluster_scope};

fn real_schema() -> serde_json::Value {
    SettingsStore::new(oxikube_assets::default_settings())
        .expect("the embedded defaults load")
        .json_schema()
}

#[test]
fn only_fields_that_support_cluster_scope_are_offered() {
    let schema = json!({
        "type": "object",
        "properties": {
            "ui_scale": { "type": ["number", "null"] },
            "read_only": { "type": ["boolean", "null"], CLUSTER_SCOPED_KEY: true },
            "prometheus": { "$ref": "#/$defs/Prom", CLUSTER_SCOPED_KEY: true },
            "terminal": { "$ref": "#/$defs/Term" }
        },
        "$defs": {
            "Prom": { "type": "object", "properties": { "url": { "type": ["string", "null"] } } },
            "Term": { "type": "object", "properties": { "font_size": { "type": ["number", "null"] } } }
        }
    });
    let model = cluster_page_model(&schema);
    assert!(model.find_field("read_only").is_some());
    assert!(model.find_field("prometheus.url").is_some());
    assert!(model.find_field("ui_scale").is_none());
    assert!(model.find_field("terminal.font_size").is_none());
    assert_eq!(model.sections()[0].title, "Cluster");
    assert!(supports_cluster_scope(&schema, "read_only"));
    assert!(supports_cluster_scope(&schema, "prometheus.url"));
    assert!(!supports_cluster_scope(&schema, "terminal.font_size"));
}

#[test]
fn the_real_schema_offers_the_cluster_fields_and_nothing_global() {
    let model = cluster_page_model(&real_schema());
    for path in [
        "display_name",
        "colour",
        "read_only",
        "default_namespace",
        "terminal_cwd",
        "node_shell_image",
        "node_shell_pull_secret",
        "accessible_namespaces",
        "exec_interactivity",
        "exec_in_read_only",
        "prometheus.url",
    ] {
        assert!(model.find_field(path).is_some(), "{path} is per cluster");
    }
    for path in [
        "ui_scale",
        "terminal.font_size",
        "editor.tab_size",
        "confirm_quit",
    ] {
        assert!(model.find_field(path).is_none(), "{path} is global");
    }
}

#[test]
fn a_schema_without_cluster_fields_gives_an_empty_model() {
    let schema = json!({ "type": "object", "properties": { "ui_scale": { "type": "number" } } });
    assert_eq!(cluster_page_model(&schema).field_count(), 0);
}

#[test]
fn the_list_is_the_catalog_plus_blocks_the_catalog_does_not_list() {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
    let (prod, gone) = (id("prod"), id("gone"));
    store
        .set_user_settings(&format!(
            r#"{{ "clusters": {{
                "{prod}": {{ "display_name": "Production", "read_only": true }},
                "{gone}": {{ "colour": "red" }}
            }} }}"#
        ))
        .expect("valid settings");
    let rows = choices(&[entry("prod"), entry("dev")], Some(&store));
    let ids: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[..2], [prod, id("dev")]);
    assert_eq!(ids[2], gone);

    // A display name wins over the context name; the context stays in the detail line.
    assert_eq!(rows[0].label(Some(&store)), "Production");
    assert!(rows[0].detail(Some(&store)).starts_with("prod"));
    assert_eq!(rows[0].override_count(Some(&store)), 2);
    assert_eq!(rows[1].label(Some(&store)), "dev");
    assert_eq!(rows[1].override_count(Some(&store)), 0);
    assert!(
        rows[2]
            .detail(Some(&store))
            .contains("not in your kubeconfig")
    );
}
