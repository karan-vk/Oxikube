//! The notification keys of the `events` settings (E14-S09) through the real store (default.json,
//! the user's file, `clusters.<id>`), the generated schema, and into a running
//! `NotificationService`.

use std::sync::Arc;
use std::time::Duration;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_app::notifications::{DEFAULT_WATCHED_KINDS, NotificationRules, NotificationService};
use oxikube_app::{ClusterSessionManager, EventService, EventsConfig};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_settings::{SettingsDiagnostic, SettingsStore};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use serde_json::{Value, json};

use super::*;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
    store.set_user_settings(user).expect("valid settings");
    store
}

fn cluster(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

fn kinds(list: &[Arc<str>]) -> Vec<&str> {
    list.iter().map(|k| &**k).collect()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn rules(user: &str) -> NotificationRules {
    EventsSettings::notification_rules(&store(user), NotificationRules::default())
}

#[test]
fn default_json_and_the_code_defaults_agree() {
    let settings = store("{}").get::<EventsSettings>(None).clone();
    assert_eq!(settings, EventsSettings::default());
    assert_eq!(kinds(&settings.watched_kinds), DEFAULT_WATCHED_KINDS);
    assert!(settings.ignore_reasons.is_empty());
    assert!(!settings.os_notifications, "OS notifications are opt-in");

    let defaults = NotificationRules::default();
    let from_json = rules("{}");
    assert_eq!(
        from_json, defaults,
        "the shipped settings are the centre's own defaults"
    );
    assert_eq!(from_json.object_interval, secs(300));
    assert_eq!(from_json.group_threshold, 3);
    assert_eq!(from_json.group_window, secs(60));
    assert_eq!(from_json.max_raised, 5);
    assert_eq!(from_json.raise_window, secs(30));

    let json = oxikube_settings::jsonc::parse_jsonc_object(oxikube_assets::default_settings())
        .expect("default.json parses");
    let block = &json["events"];
    assert_eq!(block["watched_kinds"], json!(DEFAULT_WATCHED_KINDS));
    assert_eq!(block["ignore_reasons"], json!([]));
    assert_eq!(block["rate_limit"]["per_object"], "5m");
    assert_eq!(block["rate_limit"]["per_reason"], 3);
}

#[test]
fn a_user_value_overrides_the_default_and_keeps_the_rest() {
    let user = r#"{ "events": { "watched_kinds": ["Service", " pod ", "POD", ""],
        "ignore_reasons": ["BackOff", "BackOff", "backoff"],
        "rate_limit": { "per_object": "10m", "max_raised": 2 } } }"#;
    let settings = store(user).get::<EventsSettings>(None).clone();
    assert_eq!(
        kinds(&settings.watched_kinds),
        ["Service", "pod"],
        "trimmed, no repeats"
    );
    assert_eq!(
        kinds(&settings.ignore_reasons),
        ["BackOff", "backoff"],
        "reasons are exact"
    );
    assert_eq!(settings.rate_limit.per_object, secs(600));
    assert_eq!(settings.rate_limit.max_raised, 2);
    assert_eq!(
        settings.rate_limit.per_reason, 3,
        "the rest keeps its default"
    );
    assert_eq!(settings.buffer, oxikube_app::events::DEFAULT_EVENTS_BUFFER);

    let rules = rules(user);
    assert_eq!(kinds(&rules.watched_kinds), ["Service", "pod"]);
    assert_eq!(rules.object_interval, secs(600));
    assert_eq!(
        rules.dedupe_window,
        NotificationRules::default().dedupe_window
    );
}

#[test]
fn an_empty_list_notifies_about_nothing() {
    let rules = rules(r#"{ "events": { "watched_kinds": [] } }"#);
    assert!(rules.watched_kinds.is_empty());
}

#[test]
fn numbers_and_durations_out_of_range_are_clamped_and_bad_text_falls_back() {
    let limit = |text: &str| {
        let user = format!(r#"{{ "events": {{ "rate_limit": {text} }} }}"#);
        store(&user).get::<EventsSettings>(None).rate_limit
    };
    let low = limit(
        r#"{ "per_object": "0s", "per_reason": 0, "group_window": "0m", "max_raised": -4, "raise_window": "0s" }"#,
    );
    assert_eq!(low.per_object, secs(1));
    assert_eq!(low.per_reason, 1);
    assert_eq!(low.group_window, secs(1));
    assert_eq!(low.max_raised, 1);
    assert_eq!(low.raise_window, secs(1));
    let high = limit(
        r#"{ "per_object": "9000d", "per_reason": 1000000, "group_window": "5d", "max_raised": 101, "raise_window": "2h" }"#,
    );
    assert_eq!(high.per_object, secs(86_400));
    assert_eq!(high.per_reason, 100);
    assert_eq!(high.group_window, secs(3_600));
    assert_eq!(high.max_raised, 100);
    assert_eq!(high.raise_window, secs(3_600));
    let edge = limit(r#"{ "per_object": " 90 s ", "per_reason": 100, "group_window": "1h" }"#);
    assert_eq!(edge.per_object, secs(90));
    assert_eq!(edge.per_reason, 100);
    assert_eq!(edge.group_window, secs(3_600));

    let bad = limit(r#"{ "per_object": "soon", "group_window": "1 week", "raise_window": "" }"#);
    assert_eq!(
        bad,
        RateLimit::default(),
        "text that is no duration is the default"
    );
    assert_eq!(limit("{}"), RateLimit::default());
}

#[test]
fn lists_are_capped() {
    let many: Vec<String> = (0..300).map(|i| format!("Kind{i}")).collect();
    let user = json!({ "events": { "watched_kinds": many, "ignore_reasons": many } });
    let settings = store(&user.to_string()).get::<EventsSettings>(None).clone();
    assert_eq!(settings.watched_kinds.len(), MAX_WATCHED_KINDS);
    assert_eq!(settings.ignore_reasons.len(), MAX_IGNORED_REASONS);
}

#[test]
fn a_value_of_the_wrong_type_is_a_diagnostic_and_the_defaults_stand() {
    for (key, value) in [
        ("os_notifications", json!("yes")),
        ("os_notifications", json!(1)),
        ("watched_kinds", json!("Pod")),
        ("ignore_reasons", json!({ "BackOff": true })),
        ("rate_limit", json!(5)),
    ] {
        let user = json!({ "events": { key: value } }).to_string();
        let store = store(&user);
        let settings = store.get::<EventsSettings>(None).clone();
        assert_eq!(
            settings,
            EventsSettings::default(),
            "events.{key} of the wrong type"
        );
        assert!(
            store
                .diagnostics()
                .iter()
                .any(|d| matches!(d, SettingsDiagnostic::InvalidValue { .. })),
            "events.{key}: {:?}",
            store.diagnostics()
        );
    }
    // Notifications stay off whatever else the file says.
    let store = store(r#"{ "events": { "os_notifications": "on", "buffer": 5000 } }"#);
    assert!(!store.get::<EventsSettings>(None).os_notifications);
}

#[test]
fn a_cluster_overrides_the_user_who_overrides_the_default() {
    let (prod, dev, lab) = (cluster("prod"), cluster("dev"), cluster("lab"));
    let user = json!({
        "events": {
            "watched_kinds": ["Pod"],
            "ignore_reasons": ["BackOff"],
            "os_notifications": true,
            "rate_limit": { "max_raised": 8 },
        },
        "clusters": {
            prod.to_string(): { "events": {
                "watched_kinds": ["Deployment", "Node"],
                "os_notifications": false,
                "rate_limit": { "max_raised": 1 },
            } },
            dev.to_string(): { "events": { "ignore_reasons": [] } },
            // Same as the user's: no override to carry.
            lab.to_string(): { "events": { "watched_kinds": ["Pod"] } },
            "not a cluster id": { "events": { "watched_kinds": ["Node"] } },
        },
    })
    .to_string();
    let store = store(&user);
    assert_eq!(
        kinds(&store.get::<EventsSettings>(None).watched_kinds),
        ["Pod"]
    );

    let rules = EventsSettings::notification_rules(&store, NotificationRules::default());
    assert_eq!(kinds(&rules.watched_kinds), ["Pod"]);
    assert_eq!(kinds(&rules.ignored_reasons), ["BackOff"]);
    assert_eq!(
        rules.max_raised, 8,
        "the rate limit is the user's, never a cluster's"
    );

    assert_eq!(
        kinds(rules.watched_kinds_for(&prod)),
        ["Deployment", "Node"]
    );
    assert_eq!(
        kinds(rules.ignored_reasons_for(&prod)),
        ["BackOff"],
        "inherited"
    );
    assert!(!rules.os_allowed(&prod));
    assert!(rules.os_allowed(&dev));
    assert!(rules.ignored_reasons_for(&dev).is_empty());
    assert_eq!(kinds(rules.watched_kinds_for(&dev)), ["Pod"]);
    assert_eq!(kinds(rules.watched_kinds_for(&cluster("other"))), ["Pod"]);
    assert_eq!(
        rules.clusters.len(),
        2,
        "lab says what the user says: {:?}",
        rules.clusters
    );
}

#[test]
fn a_cluster_cannot_turn_the_desktop_on() {
    let prod = cluster("prod");
    let user =
        json!({ "clusters": { prod.to_string(): { "events": { "os_notifications": true } } } })
            .to_string();
    let rules = rules(&user);
    // The cluster says yes, the user's opt-in (off) is the master switch of the OS service.
    assert!(rules.os_allowed(&prod));
    assert!(!store(&user).get::<EventsSettings>(None).os_notifications);
}

/// The `events` object of the generated schema.
fn schema_properties() -> Value {
    let schema: Value = serde_json::from_str(oxikube_assets::settings_schema()).expect("schema");
    let reference = schema["properties"]["events"]["$ref"]
        .as_str()
        .expect("a $ref");
    let name = reference.rsplit('/').next().expect("a name");
    schema["$defs"][name]["properties"].clone()
}

#[test]
fn the_schema_names_every_key_with_its_bounds() {
    let schema: Value = serde_json::from_str(oxikube_assets::settings_schema()).expect("schema");
    let events = schema_properties();
    // The keys other stories added are listed in the defaults test; these are this story's, so a
    // rename fails here and has to be explained in review.
    for key in [
        "buffer",
        "os_notifications",
        "watched_kinds",
        "ignore_reasons",
        "rate_limit",
    ] {
        let entry = &events[key];
        assert!(
            entry.is_object(),
            "settings.schema.json has no events.{key}"
        );
        assert!(
            entry["description"].as_str().is_some_and(|d| d.len() > 20),
            "events.{key} has no description"
        );
    }
    assert_eq!(events["buffer"]["minimum"], 100);
    assert_eq!(events["buffer"]["maximum"], 50_000);
    assert_eq!(events["watched_kinds"]["maxItems"], 64);
    assert_eq!(events["watched_kinds"]["items"]["type"], "string");
    assert_eq!(events["watched_kinds"]["items"]["minLength"], 1);
    assert_eq!(events["ignore_reasons"]["maxItems"], 256);

    let limit = &schema["$defs"]["RateLimitContent"];
    assert_eq!(
        limit["additionalProperties"], false,
        "a misspelt key is reported"
    );
    let props = limit["properties"].as_object().expect("properties");
    let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "group_window",
            "max_raised",
            "per_object",
            "per_reason",
            "raise_window"
        ]
    );
    for key in ["per_reason", "max_raised"] {
        assert_eq!(props[key]["minimum"], 1, "{key}");
        assert_eq!(props[key]["maximum"], 100, "{key}");
    }
    let pattern = regex_lite_check(props["per_object"]["pattern"].as_str().expect("a pattern"));
    assert!(pattern("30s") && pattern("5m") && pattern("1d"));
    assert!(!pattern("soon") && !pattern("5") && !pattern("5w"));
}

/// A check of the three-part pattern `^\s*\d+\s*[smhd]\s*$` without a regex dependency.
fn regex_lite_check(pattern: &str) -> impl Fn(&str) -> bool {
    assert_eq!(pattern, r"^\s*\d+\s*[smhd]\s*$");
    |text: &str| {
        let t = text.trim();
        let digits = t.trim_end_matches(|c: char| "smhd".contains(c)).trim_end();
        t.len() > digits.len()
            && t.len() - digits.len() <= 1
            && !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit())
    }
}

fn centre() -> NotificationService {
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(FakeClockPort::default()),
    );
    let events = EventService::new(
        sessions.clone(),
        Arc::new(FakeClockPort::default()),
        EventsConfig::default(),
    );
    NotificationService::new(
        events,
        sessions,
        Arc::new(FakeStatePort::new()),
        Arc::new(FakeClockPort::default()),
    )
}

fn set(cx: &mut TestAppContext, text: &str) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(text).expect("valid settings");
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
fn the_centre_follows_edits_of_the_settings(cx: &mut TestAppContext) {
    cx.set_global(store("{}"));
    let service = centre();
    let _follow = cx.update(|cx| follow_notification_rules(&service, cx));
    assert_eq!(service.rules(), NotificationRules::default());

    set(
        cx,
        r#"{ "events": { "watched_kinds": ["Job"], "ignore_reasons": ["Failed"],
             "rate_limit": { "max_raised": 2, "raise_window": "10s" } } }"#,
    );
    let rules = service.rules();
    assert_eq!(kinds(&rules.watched_kinds), ["Job"]);
    assert_eq!(kinds(&rules.ignored_reasons), ["Failed"]);
    assert_eq!((rules.max_raised, rules.raise_window), (2, secs(10)));
    assert_eq!(
        rules.capacity,
        NotificationRules::default().capacity,
        "what is not a setting stays"
    );

    let prod = cluster("prod");
    let user = json!({ "clusters": { prod.to_string(): { "events": {
        "watched_kinds": ["Node"], "os_notifications": false } } } })
    .to_string();
    set(cx, &user);
    let rules = service.rules();
    assert_eq!(
        kinds(&rules.watched_kinds),
        DEFAULT_WATCHED_KINDS,
        "the Job edit is gone"
    );
    assert_eq!(kinds(rules.watched_kinds_for(&prod)), ["Node"]);
    assert!(
        rules.os_allowed(&prod),
        "off for the desktop only when the global is on"
    );

    let user = json!({ "events": { "os_notifications": true }, "clusters": {
        prod.to_string(): { "events": { "os_notifications": false } } } })
    .to_string();
    set(cx, &user);
    assert!(!service.os_allowed(&prod));
    assert!(service.os_allowed(&cluster("dev")));

    set(cx, "{}");
    assert_eq!(
        service.rules(),
        NotificationRules::default(),
        "back to the defaults"
    );
}

#[gpui::test]
fn an_unrelated_edit_does_not_touch_the_rules(cx: &mut TestAppContext) {
    cx.set_global(store("{}"));
    let service = centre();
    let _follow = cx.update(|cx| follow_notification_rules(&service, cx));
    set(cx, r#"{ "events": { "watched_kinds": ["Job"] } }"#);
    let before = service.rules();
    set(
        cx,
        r#"{ "events": { "watched_kinds": ["Job"] }, "logs": { "wrap": true } }"#,
    );
    assert_eq!(service.rules(), before);
}
