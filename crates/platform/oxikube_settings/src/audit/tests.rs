use super::*;
use crate::store::SettingsStore;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
    store.register_setting::<AuditSettings>();
    store.set_user_settings(user).unwrap();
    store
}

#[test]
fn the_default_is_ninety_days_and_default_json_says_so() {
    let store = store("{}");
    assert_eq!(store.get::<AuditSettings>(None).retention_days, 90);
    assert!(store.diagnostics().is_empty(), "{:?}", store.diagnostics());
    let defaults: serde_json::Value =
        serde_json_lenient::from_str(oxikube_assets::default_settings()).unwrap();
    assert_eq!(
        defaults["audit"]["retention_days"],
        u64::from(DEFAULT_RETENTION_DAYS)
    );
}

#[test]
fn the_user_value_wins_zero_keeps_everything_and_big_values_are_clamped() {
    for (json, expected) in [
        (r#"{ "audit": { "retention_days": 7 } }"#, 7),
        (r#"{ "audit": { "retention_days": 0 } }"#, 0),
        (
            r#"{ "audit": { "retention_days": 99999 } }"#,
            MAX_RETENTION_DAYS,
        ),
    ] {
        assert_eq!(
            store(json).get::<AuditSettings>(None).retention_days,
            expected,
            "{json}"
        );
    }
}

#[test]
fn a_wrong_type_is_a_diagnostic_and_keeps_the_default() {
    let store = store(r#"{ "audit": { "retention_days": "soon" } }"#);
    assert_eq!(store.get::<AuditSettings>(None).retention_days, 90);
    assert!(!store.diagnostics().is_empty());
}
