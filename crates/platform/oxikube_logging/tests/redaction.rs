//! Captured, real formatted tracing output must never contain the fixture secrets.

use oxikube_logging::{
    DEFAULT_DIRECTIVES, RedactingFields, RedactingMakeWriter, redacting_json_layer, redacting_layer,
};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

// Fake secrets. Nothing here is a real credential.
const TOKEN: &str = "s3cr3t-bearer-token-0123456789";
const KEY_DATA: &str = "LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLWZha2VrZXlkYXRh";
const JWT: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZha2UifQ.eyJzdWIiOiJzeXN0ZW06c2VydmljZWFjY291bnQ6ZGVmYXVsdDpmYWtlIn0.c2lnbmF0dXJlLWZha2U";
const SECRET_VALUE: &str = "cGFzc3dvcmQtZmFrZS12YWx1ZQ==";
const BEARER_IN_ERROR: &str = "abc.def.ghi";
const PASSWORD: &str = "hunter2hunter2";
const ALL: &[&str] = &[
    TOKEN,
    KEY_DATA,
    JWT,
    SECRET_VALUE,
    BEARER_IN_ERROR,
    PASSWORD,
];

/// A `MakeWriter` collecting everything written.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

/// What a careless `#[derive(Debug)]` on a credential struct would print.
#[derive(Debug)]
#[allow(dead_code)]
struct FakeConfig {
    cluster_url: String,
    token: Option<String>,
    client_key_data: Option<String>,
}

fn log_secrets() {
    let config = FakeConfig {
        cluster_url: "https://10.0.0.1:6443".into(),
        token: Some(TOKEN.into()),
        client_key_data: Some(KEY_DATA.into()),
    };
    tracing::info!(?config, "built client");
    tracing::warn!("request failed: Authorization: Bearer {BEARER_IN_ERROR}");
    tracing::info!("exec plugin returned {JWT}");
    let secret_data: BTreeMap<&str, &str> = [("password", SECRET_VALUE)].into();
    tracing::info!(kind = "Secret", data = ?secret_data, "fetched secret");
    tracing::error!(token = TOKEN, password = PASSWORD, "login");
}

fn assert_no_secrets(output: &str) {
    for secret in ALL {
        assert!(!output.contains(secret), "{secret:?} leaked:\n{output}");
    }
    assert!(output.contains("[redacted]"), "no marker:\n{output}");
}

fn assert_clean(output: &str) {
    assert_no_secrets(output);
    assert!(
        output.contains("https://10.0.0.1:6443"),
        "host dropped:\n{output}"
    );
    assert!(
        output.contains("built client"),
        "message dropped:\n{output}"
    );
}

#[test]
fn layer_output_contains_no_secrets() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_secrets);
    assert_clean(&capture.text());
}

#[test]
fn writer_alone_is_a_sufficient_backstop() {
    // Default field formatting; only the writer redacts.
    let capture = Capture::default();
    let layer = fmt::layer()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter::new(capture.clone()));
    tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), log_secrets);
    assert_clean(&capture.text());
}

#[test]
fn fields_formatter_alone_redacts_by_name() {
    // Plain writer; only the fields formatter redacts.
    let capture = Capture::default();
    let layer = fmt::layer()
        .with_ansi(false)
        .fmt_fields(RedactingFields)
        .with_writer(capture.clone());
    tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
        tracing::info!(
            id_token = "innocuous-looking",
            password = "plain",
            user = "bob",
            "login"
        );
    });
    let out = capture.text();
    assert!(
        !out.contains("innocuous-looking") && !out.contains("plain"),
        "{out}"
    );
    assert!(
        out.contains("id_token=[redacted]") && out.contains("user=\"bob\""),
        "{out}"
    );
}

#[test]
fn third_party_style_header_dump_is_scrubbed() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry()
        .with(redacting_layer(capture.clone()).with_filter(EnvFilter::new("trace")));
    let headers: BTreeMap<&str, String> = [
        ("accept", "application/json".to_owned()),
        ("authorization", format!("Bearer {TOKEN}")),
        ("user-agent", "oxikube/0.0".to_owned()),
    ]
    .into();
    tracing::subscriber::with_default(subscriber, || {
        // What hyper / tower-http emit at debug and trace: a Debug-printed header map.
        tracing::debug!(target: "hyper::proto::h1::io", ?headers, "flushed request");
        tracing::trace!(target: "tower_http::trace", "request headers: {headers:?}");
    });
    let out = capture.text();
    assert!(!out.contains(TOKEN), "{out}");
    assert!(
        out.contains("application/json") && out.contains("oxikube/0.0"),
        "{out}"
    );
    assert_eq!(out.matches("[redacted]").count(), 2, "{out}");
}

#[test]
fn span_fields_are_redacted() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("connect", token = TOKEN, context = "prod");
        let _guard = span.enter();
        tracing::info!("connected");
    });
    let out = capture.text();
    assert!(!out.contains(TOKEN), "{out}");
    assert!(
        out.contains("connect{token=[redacted] context=\"prod\"}"),
        "{out}"
    );
}

#[test]
fn json_layer_output_contains_no_secrets() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_json_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_secrets);
    let out = capture.text();
    assert_no_secrets(&out);
    // Still one valid JSON object per line.
    for line in out.lines() {
        assert!(
            serde_json::from_str::<serde_json::Value>(line).is_ok(),
            "invalid JSON: {line}"
        );
    }
}

#[test]
fn json_layer_scrubs_secret_data_with_non_secret_looking_keys() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_json_layer(capture.clone()));
    let map: BTreeMap<&str, &str> = [("user", "YWRtaW4="), ("tls.key", "ZmFrZWtleQ==")].into();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(data = ?map, "fetched secret");
        let headers: BTreeMap<&str, &str> = [("Authorization", "ApiKey abc123xyz")].into();
        tracing::info!(?headers, "sent");
        tracing::warn!(
            "Authorization: Digest username=\"bob\", nonce=\"abc123\", response=\"deadbeefcafe\""
        );
        tracing::warn!("Authorization: OAuth oauth_consumer_key=\"k\", oauth_signature=\"sigsig\"");
    });
    let out = capture.text();
    for secret in [
        "YWRtaW4=",
        "ZmFrZWtleQ==",
        "abc123xyz",
        "abc123",
        "deadbeefcafe",
        "sigsig",
    ] {
        assert!(!out.contains(secret), "{secret:?} leaked:\n{out}");
    }
    for line in out.lines() {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
        assert!(parsed.is_ok(), "invalid JSON after scrubbing: {line}");
    }
}

/// Logs `SECRETV` under each field name, once as a scalar and once as a `Debug` map.
macro_rules! log_named {
    ($($name:literal),+ $(,)?) => {{
        let map: BTreeMap<&str, &str> = [("user", "SECRETV")].into();
        $(
            tracing::info!($name = "SECRETV", "scalar");
            tracing::info!($name = ?map, "map");
        )+
    }};
}

#[test]
fn json_layer_redacts_every_sensitive_field_name() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_json_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, || {
        // Every `SENSITIVE_FIELDS` entry, plus names that only a suffix makes sensitive.
        log_named!(
            "client-key-data",
            "client_key_data",
            "client-certificate-data",
            "client_certificate_data",
            "client-secret",
            "client_secret",
            "secret-data",
            "secret_data",
            "string-data",
            "string_data",
            "stringdata",
            "stringData",
            "id_token",
            "db.password",
            "passwd",
            "proxy_authorization",
            "http.authorization",
            "aws_secret",
            "x-api-key",
            "api_key",
            "apikey",
        );
        tracing::info!(
            proxy_authorization = "Basic YWRtaW46c2VjcmV0UFc=",
            "upstream"
        );
    });
    let out = capture.text();
    assert_eq!(out.lines().count(), 43, "{out}");
    for secret in ["SECRETV", "YWRtaW46c2VjcmV0UFc="] {
        assert!(!out.contains(secret), "{secret:?} leaked:\n{out}");
    }
    for name in [
        "secret_data",
        "string_data",
        "stringData",
        "proxy_authorization",
    ] {
        assert!(
            out.contains(&format!(r#""{name}":"[redacted]""#)),
            "{name} not redacted in place:\n{out}"
        );
    }
    for line in out.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("valid JSON after scrubbing");
    }
}

const MANIFEST: &str = "apiVersion: v1\nkind: Secret\ndata:\n  tls.key: TLSKEYVALUE\n";

fn log_manifest() {
    tracing::info!("manifest:\n{MANIFEST}");
    tracing::info!(manifest = MANIFEST, "as str");
    tracing::info!(manifest = ?MANIFEST, "as debug");
    tracing::info!(manifest = %MANIFEST, "as display");
}

#[test]
fn json_layer_redacts_a_secret_manifest_behind_escaped_newlines() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_json_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_manifest);
    let out = capture.text();
    assert_eq!(out.lines().count(), 4, "{out}");
    assert!(!out.contains("TLSKEYVALUE"), "{out}");
    assert_eq!(out.matches("tls.key: [redacted]").count(), 4, "{out}");
}

#[test]
fn text_layer_redacts_a_secret_manifest_in_str_and_debug_fields() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_manifest);
    let out = capture.text();
    assert!(!out.contains("TLSKEYVALUE"), "{out}");
    assert_eq!(out.matches("tls.key: [redacted]").count(), 4, "{out}");
    // A `&str` field is still printed quoted, as the default formatter does.
    assert!(out.contains(r#"manifest="apiVersion: v1\nkind"#), "{out}");
}

#[test]
fn fields_formatter_alone_redacts_a_str_manifest() {
    // Plain writer: `record_str` must redact before `Debug` quoting escapes the newlines.
    let capture = Capture::default();
    let layer = fmt::layer()
        .with_ansi(false)
        .fmt_fields(RedactingFields)
        .with_writer(capture.clone());
    tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
        tracing::info!(manifest = MANIFEST, "as str");
    });
    let out = capture.text();
    assert!(!out.contains("TLSKEYVALUE"), "{out}");
    assert!(out.contains(r#"tls.key: [redacted]\n""#), "{out}");
}

/// A `stringData` block whose first value is a double-quoted string holding a `\n` escape,
/// followed by an entry whose name is not sensitive on its own.
const QUOTED_ESCAPE_MANIFEST: &str = "stringData:\n  config: \"a\\nb\"\n  tls.key: SECRETVALUE4\n";

fn log_quoted_escape_manifest() {
    tracing::info!("manifest:\n{QUOTED_ESCAPE_MANIFEST}");
    tracing::info!(manifest = QUOTED_ESCAPE_MANIFEST, "as str");
    tracing::info!(manifest = ?QUOTED_ESCAPE_MANIFEST, "as debug");
}

fn assert_quoted_escape_manifest_redacted(out: &str) {
    assert!(!out.contains("SECRETVALUE4"), "{out}");
    // The tail of the quoted value is not left behind outside it.
    assert!(!out.contains("b\\\"") && !out.contains("\nb\""), "{out}");
    assert_eq!(out.matches("tls.key: [redacted]").count(), 3, "{out}");
}

#[test]
fn text_layer_keeps_a_block_open_across_an_escaped_newline_in_a_quoted_value() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_quoted_escape_manifest);
    assert_quoted_escape_manifest_redacted(&capture.text());
}

#[test]
fn json_layer_keeps_a_block_open_across_an_escaped_newline_in_a_quoted_value() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_json_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_quoted_escape_manifest);
    let out = capture.text();
    assert_quoted_escape_manifest_redacted(&out);
    for line in out.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("valid JSON after scrubbing");
    }
}

/// What a `{:#?}` dump of an object holding a Secret manifest would print.
#[derive(Debug)]
#[allow(dead_code)]
struct AppliedSpec {
    manifest: &'static str,
    replicas: u32,
}

/// A pretty-printed object whose manifest string opens with the `data:` header, so the
/// header's line starts with the outer indent and the field name.
fn log_pretty_nested_manifest() {
    let spec = AppliedSpec {
        manifest: "data:\n  tls.key: NESTEDSECRET1\n",
        replicas: 2,
    };
    tracing::info!("applying {spec:#?}");
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "spec": { "manifest": "data:\n  tls.key: NESTEDSECRET2\n" },
    }))
    .expect("serialise");
    tracing::info!("request body: {body}");
    tracing::info!(body = body.as_str(), "as str");
}

fn assert_pretty_nested_manifest_redacted(out: &str) {
    assert!(!out.contains("NESTEDSECRET"), "{out}");
    assert_eq!(out.matches("tls.key: [redacted]").count(), 3, "{out}");
    assert!(out.contains("replicas: 2"), "{out}");
}

#[test]
fn text_layer_redacts_a_manifest_nested_in_pretty_debug_and_json() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_pretty_nested_manifest);
    assert_pretty_nested_manifest_redacted(&capture.text());
}

#[test]
fn json_layer_redacts_a_manifest_nested_in_pretty_debug_and_json() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(redacting_json_layer(capture.clone()));
    tracing::subscriber::with_default(subscriber, log_pretty_nested_manifest);
    let out = capture.text();
    assert_pretty_nested_manifest_redacted(&out);
    for line in out.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("valid JSON after scrubbing");
    }
}

#[test]
fn json_layer_keeps_metadata_after_a_block_with_no_trailing_newline() {
    let capture = Capture::default();
    let layer = redacting_json_layer(capture.clone()).with_line_number(true);
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("req", request_id = 7);
        let _guard = span.enter();
        tracing::info!("data:\n  tls.key: TLSKEY");
        tracing::info!(manifest = "data:\n  tls.key: TLSKEY", "as str");
        tracing::info!(manifest = ?"data:\n  tls.key: TLSKEY", "as debug");
    });
    let out = capture.text();
    assert_eq!(out.lines().count(), 3, "{out}");
    assert!(!out.contains("TLSKEY"), "{out}");
    for line in out.lines() {
        let record: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("invalid JSON ({e}): {line}"));
        assert_eq!(record["target"], "redaction", "{line}");
        assert!(record["line_number"].is_u64(), "{line}");
        assert_eq!(record["span"]["name"], "req", "{line}");
        assert_eq!(record["span"]["request_id"], 7, "{line}");
        assert_eq!(record["spans"][0]["name"], "req", "{line}");
    }
}

#[test]
fn trace_level_output_contains_no_secrets() {
    // RUST_LOG=trace equivalent: every event at every level reaches the writer.
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry()
        .with(redacting_layer(capture.clone()).with_filter(EnvFilter::new("trace")));
    tracing::subscriber::with_default(subscriber, || {
        log_secrets();
        tracing::trace!("trace dump: Authorization: Bearer {TOKEN} {JWT}");
        tracing::debug!("debug dump: client-key-data: {KEY_DATA}");
        assert!(tracing::enabled!(Level::TRACE));
    });
    let out = capture.text();
    assert_no_secrets(&out);
    assert!(
        out.contains("TRACE") && out.contains("DEBUG"),
        "levels missing:\n{out}"
    );
}

#[test]
fn default_directives_never_enable_debug_or_trace() {
    let lower = DEFAULT_DIRECTIVES.to_ascii_lowercase();
    assert!(
        !lower.contains("trace") && !lower.contains("debug"),
        "{DEFAULT_DIRECTIVES}"
    );
    assert_eq!(
        EnvFilter::new(DEFAULT_DIRECTIVES).max_level_hint(),
        Some(LevelFilter::INFO)
    );
}

#[test]
fn writer_survives_invalid_utf8_and_reports_input_length() {
    let capture = Capture::default();
    let make = RedactingMakeWriter::new(capture.clone());
    let mut writer = make.make_writer();
    let bytes = b"token=abcdef123456 \xff\xfe tail";
    assert_eq!(writer.write(bytes).unwrap(), bytes.len());
    writer.flush().unwrap();
    let out = capture.text();
    assert!(
        !out.contains("abcdef123456") && out.contains("tail"),
        "{out}"
    );
}
