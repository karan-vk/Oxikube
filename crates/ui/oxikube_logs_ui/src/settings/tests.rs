//! The `logs` settings through the real store: defaults, clamping with a warning, and the layers
//! (default.json, then the user's file, then `clusters.<id>`).

use std::io;
use std::sync::{Arc, Mutex};

use oxikube_app::logs::{DEFAULT_BUFFER_LINES, MAX_BUFFER_LINES, MIN_BUFFER_LINES};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_settings::{Settings as _, SettingsLocation, SettingsStore};
use tracing_subscriber::fmt::MakeWriter;

use super::*;

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
    store.set_user_settings(user).expect("valid settings");
    store
}

fn cluster(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

fn content(buffer: Option<i64>, tail: Option<i64>) -> LogsSettings {
    LogsSettings::from_content(LogsContent {
        buffer_lines: buffer,
        default_tail: tail,
        ..LogsContent::default()
    })
}

#[test]
fn default_json_spells_out_every_default() {
    let store = store("{}");
    let settings = store.get::<LogsSettings>(None);
    assert_eq!(
        *settings,
        LogsSettings {
            buffer_lines: DEFAULT_BUFFER_LINES,
            default_tail: DEFAULT_TAIL,
            wrap: false,
            timestamps: false,
            json_auto_detect: true,
            max_streams: DEFAULT_MAX_STREAMS,
            reconnect_retries: DEFAULT_RECONNECT_RETRIES,
        },
        "assets/settings/default.json and the code's defaults disagree"
    );
    // Every key is written out in default.json (so the documentation sits next to the value).
    let defaults = oxikube_settings::jsonc::parse_jsonc_object(oxikube_assets::default_settings())
        .expect("default.json parses");
    let logs = defaults["logs"].as_object().expect("a logs block");
    for key in [
        "buffer_lines",
        "default_tail",
        "wrap",
        "timestamps",
        "json_auto_detect",
        "max_streams",
        "reconnect_retries",
    ] {
        assert!(logs.contains_key(key), "default.json has no logs.{key}");
    }
}

#[test]
fn the_shipped_defaults_match_what_s02_drew() {
    // No behaviour change when the settings land: unwrapped, no timestamps, the 1 000-line tail.
    let view = crate::view::ViewOptions::default();
    let settings = LogsSettings::from_content(LogsContent::default());
    assert_eq!(
        (view.wrap, view.timestamps),
        (settings.wrap, settings.timestamps)
    );
    assert_eq!(i64::from(settings.default_tail), crate::view::TAIL_LINES);
}

#[test]
fn out_of_range_values_are_clamped() {
    assert_eq!(content(Some(2_000), Some(50)).buffer_lines, 2_000);
    assert_eq!(content(Some(2_000), Some(50)).default_tail, 50);
    assert_eq!(content(Some(0), Some(0)).buffer_lines, MIN_BUFFER_LINES);
    assert_eq!(content(Some(0), Some(0)).default_tail, MIN_DEFAULT_TAIL);
    assert_eq!(content(Some(-5), Some(-5)).buffer_lines, MIN_BUFFER_LINES);
    assert_eq!(content(Some(-5), Some(-5)).default_tail, MIN_DEFAULT_TAIL);
    assert_eq!(
        content(Some(i64::MAX), Some(i64::MAX)).buffer_lines,
        MAX_BUFFER_LINES
    );
    assert_eq!(
        content(Some(i64::MAX), Some(i64::MAX)).default_tail,
        MAX_DEFAULT_TAIL
    );
    assert_eq!(content(None, None).buffer_lines, DEFAULT_BUFFER_LINES);
    let retries = |n| {
        LogsSettings::from_content(LogsContent {
            reconnect_retries: Some(n),
            ..LogsContent::default()
        })
        .reconnect_retries
    };
    assert_eq!(retries(0), 0, "0 never reconnects");
    assert_eq!(retries(-3), 0);
    assert_eq!(retries(7), 7);
    assert_eq!(retries(1_000), MAX_RECONNECT_RETRIES);
}

#[test]
fn a_clamped_value_is_resolved_not_refused() {
    // A silly value through the real store: the block still loads, the rest of it stands.
    let store =
        store(r#"{ "logs": { "buffer_lines": 3, "default_tail": 9000000, "wrap": true } }"#);
    let settings = store.get::<LogsSettings>(None);
    assert_eq!(settings.buffer_lines, MIN_BUFFER_LINES);
    assert_eq!(settings.default_tail, MAX_DEFAULT_TAIL);
    assert!(settings.wrap, "the other keys of the block are kept");
    assert!(
        store.diagnostics().is_empty(),
        "clamping is not a settings error: {:?}",
        store.diagnostics()
    );
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn clamping_warns_naming_the_key() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        content(Some(7), None);
        content(Some(2_000), Some(10));
    });
    let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(
        text.matches("WARN").count(),
        1,
        "only the bad value warns: {text}"
    );
    assert!(
        text.contains("logs.buffer_lines") && text.contains("value=7"),
        "{text}"
    );
}

#[test]
fn a_cluster_overrides_the_user_which_overrides_the_default() {
    let prod = cluster("prod");
    let dev = cluster("dev");
    let user = format!(
        r#"{{
            "logs": {{ "buffer_lines": 20000, "wrap": true }},
            "clusters": {{
                "{}": {{ "logs": {{ "buffer_lines": 200000, "timestamps": true }} }}
            }}
        }}"#,
        prod.as_str()
    );
    let store = store(&user);
    let global = store.get::<LogsSettings>(None);
    assert_eq!(global.buffer_lines, 20_000, "user over default");
    assert!(global.wrap && !global.timestamps);

    let in_prod = store.get::<LogsSettings>(Some(SettingsLocation { cluster: &prod }));
    assert_eq!(in_prod.buffer_lines, 200_000, "cluster over user");
    assert!(in_prod.timestamps, "the cluster's own key");
    assert!(
        in_prod.wrap,
        "keys the cluster does not set come from the user's layer"
    );

    let in_dev = store.get::<LogsSettings>(Some(SettingsLocation { cluster: &dev }));
    assert_eq!(
        in_dev, global,
        "a cluster without overrides reads the global value"
    );

    let overrides: Vec<_> = store.cluster_values::<LogsSettings>().collect();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].0, prod.as_str());
}

#[test]
fn a_cluster_value_out_of_range_is_clamped_too() {
    let prod = cluster("prod");
    let user = format!(
        r#"{{ "clusters": {{ "{}": {{ "logs": {{ "buffer_lines": 1 }} }} }} }}"#,
        prod.as_str()
    );
    let store = store(&user);
    let in_prod = store.get::<LogsSettings>(Some(SettingsLocation { cluster: &prod }));
    assert_eq!(in_prod.buffer_lines, MIN_BUFFER_LINES);
}

#[test]
fn the_schema_documents_every_key_with_its_range() {
    let schema = store("{}").json_schema();
    let text = serde_json::to_string(&schema).unwrap();
    let defs = &schema["$defs"]["LogsContent"]["properties"];
    for key in [
        "buffer_lines",
        "default_tail",
        "wrap",
        "timestamps",
        "json_auto_detect",
        "max_streams",
        "reconnect_retries",
    ] {
        let description = defs[key]["description"].as_str();
        assert!(
            description.is_some_and(|d| !d.is_empty()),
            "logs.{key} has no description: {text}"
        );
    }
    let json = defs["json_auto_detect"]["description"].as_str().unwrap();
    assert!(
        json.contains("JSON mode") && !json.contains("Reserved"),
        "json_auto_detect drives the JSON mode now: {json}"
    );
    let default_json =
        include_str!("../../../../platform/oxikube_assets/assets/settings/default.json");
    assert!(
        !default_json.contains("json_auto_detect  reserved"),
        "default.json must document json_auto_detect as the JSON mode's setting"
    );
    assert_eq!(
        defs["buffer_lines"]["minimum"], 100,
        "{}",
        defs["buffer_lines"]
    );
    assert_eq!(defs["buffer_lines"]["maximum"], 5_000_000);
    assert_eq!(defs["default_tail"]["minimum"], 1);
    assert_eq!(defs["default_tail"]["maximum"], 100_000);
    assert_eq!(defs["max_streams"]["minimum"], 1);
    assert_eq!(defs["max_streams"]["maximum"], 200);
}
