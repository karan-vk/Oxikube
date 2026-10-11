use oxikube_domain::utilisation::{DEFAULT_GPU_VENDORS, GpuVendors};

use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<MetricsSettings>();
    store.set_user_settings(user).unwrap();
    store
}

fn names(settings: &MetricsSettings) -> Vec<String> {
    settings
        .gpu_vendors
        .names()
        .iter()
        .map(|n| n.to_string())
        .collect()
}

#[test]
fn the_default_list_matches_default_json_and_the_domain_default() {
    let store = store("{}");
    let settings = store.get::<MetricsSettings>(None);
    assert_eq!(names(settings), DEFAULT_GPU_VENDORS);
    assert_eq!(settings.gpu_vendors, GpuVendors::default());
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());

    // The file itself carries the list, not just the code fallback.
    let defaults: serde_json::Value =
        serde_json_lenient::from_str(oxikube_assets::default_settings()).unwrap();
    let in_file: Vec<&str> = defaults["metrics"]["gpu_vendors"]
        .as_array()
        .expect("default.json has metrics.gpu_vendors")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(in_file, DEFAULT_GPU_VENDORS);
}

#[test]
fn a_user_list_replaces_the_default_and_is_normalised() {
    let store = store(
        r#"{ "metrics": { "gpu_vendors": [" example.com/fpga ", "", "example.com/fpga", "x/gpu"] } }"#,
    );
    assert_eq!(
        names(store.get::<MetricsSettings>(None)),
        ["example.com/fpga", "x/gpu"]
    );
}

#[test]
fn an_empty_list_turns_gpu_accounting_off() {
    let store = store(r#"{ "metrics": { "gpu_vendors": [] } }"#);
    assert!(store.get::<MetricsSettings>(None).gpu_vendors.is_empty());
}

#[test]
fn a_cluster_overrides_the_list() {
    let id = "3f2a9c1b7d4e8a60";
    let store = store(&format!(
        r#"{{ "metrics": {{ "gpu_vendors": ["a/gpu"] }},
            "clusters": {{ "{id}": {{ "metrics": {{ "gpu_vendors": ["b/gpu"] }} }} }} }}"#
    ));
    assert_eq!(names(store.get::<MetricsSettings>(None)), ["a/gpu"]);
    let cluster: ClusterId = id.parse().unwrap();
    let located = store.get::<MetricsSettings>(Some(crate::SettingsLocation { cluster: &cluster }));
    assert_eq!(names(located), ["b/gpu"]);
}

#[test]
fn a_wrong_type_is_reported_and_the_previous_list_stays() {
    let mut store = store(r#"{ "metrics": { "gpu_vendors": ["a/gpu"] } }"#);
    store
        .set_user_settings(r#"{ "metrics": { "gpu_vendors": "nvidia.com/gpu" } }"#)
        .unwrap();
    assert!(!store.diagnostics().is_empty());
    assert_eq!(names(store.get::<MetricsSettings>(None)), ["a/gpu"]);
}

#[test]
fn the_schema_describes_the_key() {
    let schema = serde_json::to_string(&store("{}").json_schema()).unwrap();
    assert!(
        schema.contains("\"metrics\"") && schema.contains("gpu_vendors"),
        "{schema}"
    );
}
