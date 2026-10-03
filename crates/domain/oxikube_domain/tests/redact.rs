//! Behaviour of `oxikube_domain::redact`: every pattern, idempotence, false positives, panics.

use oxikube_domain::redact::{self, MARKER, Redacted, is_sensitive_field, redact};
use proptest::prelude::*;
use std::borrow::Cow;

// Fake secrets. Nothing here is a real credential.
const JWT: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZha2UifQ.eyJzdWIiOiJzeXN0ZW06c2VydmljZWFjY291bnQ6ZGVmYXVsdDpmYWtlIn0.c2lnbmF0dXJlLWZha2U";
const KEY_DATA: &str = "LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLWZha2VrZXlkYXRh";
const CERT_DATA: &str = "LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0tZmFrZWNlcnRkYXRh";

fn assert_scrubbed(input: &str, secrets: &[&str]) -> String {
    let out = redact(input).into_owned();
    for secret in secrets {
        assert!(!out.contains(secret), "{secret:?} survived in {out:?}");
    }
    assert!(out.contains(MARKER), "no marker in {out:?}");
    assert_eq!(redact(&out), out, "not idempotent for {input:?}");
    out
}

#[test]
fn authorization_header_forms() {
    assert_scrubbed(
        "request failed: Authorization: Bearer abc.def.ghi",
        &["abc.def.ghi"],
    );
    assert_scrubbed("authorization=Basic dXNlcjpwYXNz", &["dXNlcjpwYXNz"]);
    assert_scrubbed(
        r#"headers={"authorization": "Bearer abc.def.ghi", "accept": "*/*"}"#,
        &["abc.def.ghi"],
    );
    assert_scrubbed(r#"{"Proxy-Authorization":"Basic c2VjcmV0"}"#, &["c2VjcmV0"]);
    assert_scrubbed(r#"{"authorization": Sensitive}"#, &["Sensitive"]);
    let out = redact(r#"{"authorization": "Bearer abcdefgh", "accept": "*/*"}"#);
    assert!(out.contains(r#""accept": "*/*""#), "{out}");
}

#[test]
fn authorization_with_other_schemes_and_array_forms() {
    assert_scrubbed("Authorization: ApiKey abc123xyz", &["abc123xyz"]);
    assert_scrubbed(
        r#"{"Authorization": ["Basic dXNlcjpwYXNz"]}"#,
        &["dXNlcjpwYXNz"],
    );
    assert_scrubbed("authorization: [Bearer tok123]", &["tok123"]);
    assert_scrubbed(r#"{"authorization": ["ApiKey abc123xyz"]}"#, &["abc123xyz"]);
    assert_scrubbed(
        "Authorization: AWS4-HMAC-SHA256 Credential=AKIDFAKE/20260101/s3, SignedHeaders=host, Signature=deadbeefcafe",
        &["AKIDFAKE", "deadbeefcafe"],
    );
    assert_scrubbed(
        r#"headers: {\"authorization\": \"ApiKey abc123xyz\"}"#,
        &["abc123xyz"],
    );
}

#[test]
fn quotes_inside_a_bare_secret_do_not_break_or_leak() {
    let out = assert_scrubbed(r#"password: a"b tail"#, &["a\"b", "\"b"]);
    assert_eq!(out, "password: [redacted] tail");
    let out = assert_scrubbed(r#"login failed password: a\"b tail"#, &["b tail", "a\\\"b"]);
    assert!(out.ends_with("[redacted] tail"), "{out}");
    // A trailing quote (a JSON string's closing quote) is not part of the value.
    let out = assert_scrubbed(
        r#"{"message":"retry token=abcdef123456"}"#,
        &["abcdef123456"],
    );
    assert_eq!(out, r#"{"message":"retry token=[redacted]"}"#);
}

#[test]
fn url_userinfo_password() {
    let out = assert_scrubbed(
        "HTTPS_PROXY=http://proxy-user:pr0xy-pass@proxy.corp:3128 set",
        &["pr0xy-pass"],
    );
    assert_eq!(
        out,
        "HTTPS_PROXY=http://proxy-user:[redacted]@proxy.corp:3128 set"
    );
    assert_scrubbed(
        "proxy-url: https://u:p@ss:w0rd@10.0.0.1:8080/x",
        &["p@ss:w0rd", "ss:w0rd"],
    );
    for text in [
        "ssh://git@github.com/org/repo",
        "https://example.com:8443/path@v2",
        "https://10.0.0.1:6443",
        "mailto:alice@example.com",
        "see http://host:8080 and mail bob@example.com",
    ] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn bearer_token_anywhere() {
    let out = assert_scrubbed(
        "retrying with Bearer sk.live_0123456789abcdef after 401",
        &["sk.live_0123456789abcdef"],
    );
    assert!(
        out.starts_with("retrying with Bearer ") && out.ends_with("after 401"),
        "{out}"
    );
}

#[test]
fn kubeconfig_yaml_fields() {
    let yaml = format!(
        "users:\n- name: admin\n  user:\n    client-certificate-data: {CERT_DATA}\n    client-key-data: {KEY_DATA}\n    token: s3cr3t-token-value\n    password: hunter2hunter2\n    auth-provider:\n      config:\n        id-token: {JWT}\n        refresh-token: refresh-0123456789\n        client-secret: oidc-secret-0123\n"
    );
    let out = assert_scrubbed(
        &yaml,
        &[
            CERT_DATA,
            KEY_DATA,
            "s3cr3t-token-value",
            "hunter2hunter2",
            JWT,
            "refresh-0123456789",
            "oidc-secret-0123",
        ],
    );
    assert!(out.contains("client-key-data: [redacted]"), "{out}");
    assert!(out.contains("- name: admin"), "{out}");
}

#[test]
fn json_fields_plain_and_escaped() {
    let json =
        format!(r#"{{"client-key-data":"{KEY_DATA}","token":"tok-0123456789","user":"bob"}}"#);
    let out = assert_scrubbed(&json, &[KEY_DATA, "tok-0123456789"]);
    assert!(
        out.contains(r#""token":"[redacted]""#) && out.contains(r#""user":"bob""#),
        "{out}"
    );

    let escaped = json.replace('"', "\\\"");
    let out = assert_scrubbed(&escaped, &[KEY_DATA, "tok-0123456789"]);
    assert!(out.contains(r#"\"user\":\"bob\""#), "{out}");
}

#[test]
fn key_value_and_debug_forms() {
    assert_scrubbed("connect token=abcdef123456 ok", &["abcdef123456"]);
    assert_scrubbed(
        r#"Config { token: Some("abcdef123456") }"#,
        &["abcdef123456"],
    );
    assert_scrubbed("refresh_token='abcdef123456'", &["abcdef123456"]);
    assert_scrubbed("db.password = hunter2hunter2", &["hunter2hunter2"]);
    assert_scrubbed(
        "Data { client_key_data: ByteString([1, 2, 3, 4]) }",
        &["1, 2, 3"],
    );
}

#[test]
fn jwt_shaped_strings() {
    let out = assert_scrubbed(&format!("got credential {JWT} from exec plugin"), &[JWT]);
    assert!(
        out.starts_with("got credential ") && out.ends_with(" from exec plugin"),
        "{out}"
    );
}

#[test]
fn pem_private_keys() {
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAfakefake\nZmFrZQ==\n-----END RSA PRIVATE KEY-----";
    let out = assert_scrubbed(
        &format!("key: {pem} trailing"),
        &["MIIEowIBAAKCAQEAfakefake", "ZmFrZQ=="],
    );
    assert!(out.ends_with(" trailing"), "{out}");
    // A truncated block is scrubbed to the end rather than left exposed.
    assert_scrubbed(
        "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANfake",
        &["MIIEvQIBADANfake"],
    );
    // A public certificate is not a secret.
    assert!(matches!(
        redact("-----BEGIN CERTIFICATE-----\nMIIC\n-----END CERTIFICATE-----"),
        Cow::Borrowed(_)
    ));
}

#[test]
fn secret_data_inline_maps() {
    assert_scrubbed(
        r#"secret: Secret { data: {"password": "cGFzc3dvcmQ=", "user": "YWRtaW4="} }"#,
        &["cGFzc3dvcmQ=", "YWRtaW4="],
    );
    assert_scrubbed(
        r#"{"kind":"Secret","data":{"tls.key":"ZmFrZWtleQ=="}}"#,
        &["ZmFrZWtleQ=="],
    );
    assert_scrubbed(
        r#"{\"data\":{\"tls.key\":\"ZmFrZWtleQ==\"}}"#,
        &["ZmFrZWtleQ=="],
    );
    assert_scrubbed("stringData: {password: plainpass1}", &["plainpass1"]);
    assert_scrubbed(
        "ByteString: data: {\"k\": ByteString([112, 97, 115, 115])}",
        &["112, 97"],
    );
    // Metadata maps and non-data keys are untouched.
    assert!(matches!(
        redact(r#"{"metadata": {"name": "x"}}"#),
        Cow::Borrowed(_)
    ));
}

#[test]
fn secret_data_blocks() {
    let manifest = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\ndata:\n  password: cGFzc3dvcmQ=\n  user: YWRtaW4=\ntype: Opaque\n";
    let out = assert_scrubbed(manifest, &["cGFzc3dvcmQ=", "YWRtaW4="]);
    assert!(out.contains("  name: db\n"), "{out}");
    assert!(out.contains("type: Opaque\n"), "{out}");
    assert!(out.contains("  password: [redacted]\n"), "{out}");

    let pretty = "{\n  \"data\": {\n    \"password\": \"cGFzc3dvcmQ=\",\n    \"user\": \"YWRtaW4=\"\n  },\n  \"type\": \"Opaque\"\n}";
    let out = assert_scrubbed(pretty, &["cGFzc3dvcmQ=", "YWRtaW4="]);
    assert!(out.contains("\"type\": \"Opaque\""), "{out}");
}

#[test]
fn ordinary_text_is_untouched_and_borrowed() {
    for text in [
        "Starting watcher for context prod-eu in namespace kube-system",
        "pod web-7d9c8 restarted 3 times; last state: Error (exit code 137)",
        "uid=3f2b9c1e-8a47-4d52-9e0b-6c1d2a7f5e84 resourceVersion=48213",
        "tokenizer ready; max_tokens=4096; tokens: 12; passwordless login enabled",
        "the bearer of bad news; bearer authentication is configured",
        "certificate-authority-data: LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t",
        "uid: a2VybmVsLXVpZC1ub3Qtc2VjcmV0LWp1c3QtbG9uZy1iYXNlNjQtc3RyaW5nLWhlcmU=",
        "metadata: {name: web}, dataset=ready, data was loaded",
        "tokenFile: /var/run/secrets/kubernetes.io/serviceaccount/token",
    ] {
        let out = redact(text);
        assert_eq!(out, text);
    }
}

#[test]
fn clean_text_stays_borrowed() {
    for text in ["", "plain line", "uid=3f2b9c1e", "dataset ready"] {
        assert!(matches!(redact(text), Cow::Borrowed(_)), "{text:?}");
    }
}

#[test]
fn idempotent_on_every_fixture() {
    for text in [
        "Authorization: Bearer [redacted]",
        r#""token": "[redacted]""#,
        "token=[redacted] password='[redacted]'",
        r#"{\"token\":\"[redacted]\"}"#,
        "data: {a: [redacted]}",
        "Bearer [redacted] and eyJ[redacted]",
    ] {
        let once = redact(text).into_owned();
        assert_eq!(redact(&once), once, "{text:?}");
    }
}

#[test]
fn sensitive_field_names() {
    for name in [
        "token",
        "id_token",
        "refresh-token",
        "Authorization",
        "http.authorization",
        "db.password",
        "client_key_data",
        "secret_data",
    ] {
        assert!(is_sensitive_field(name), "{name}");
    }
    for name in [
        "message",
        "max_tokens",
        "tokenizer",
        "user",
        "data",
        "uid",
        "tok",
        "",
    ] {
        assert!(!is_sensitive_field(name), "{name}");
    }
}

#[test]
fn redacted_wrapper_hides_the_value() {
    #[derive(Debug)]
    struct Creds {
        host: &'static str,
        token: Redacted<String>,
    }
    let creds = Creds {
        host: "https://10.0.0.1:6443",
        token: Redacted::new("s3cr3t".into()),
    };
    let shown = format!("{creds:?} / {}", creds.token);
    assert!(!shown.contains("s3cr3t"), "{shown}");
    assert!(
        shown.contains(MARKER) && shown.contains(creds.host),
        "{shown}"
    );
    assert_eq!(creds.token.expose(), "s3cr3t");
    assert_eq!(creds.token.into_inner(), "s3cr3t");
}

#[test]
fn pattern_catalogue_snapshot() {
    let mut listing = String::new();
    for p in redact::patterns() {
        listing.push_str(&format!(
            "{}\n  {}\n  {}\n\n",
            p.name, p.description, p.source
        ));
    }
    listing.push_str(&format!(
        "sensitive field names: {:?}\nsensitive field suffixes: {:?}\n",
        redact::SENSITIVE_FIELDS,
        redact::SENSITIVE_SUFFIXES
    ));
    insta::assert_snapshot!("patterns", listing);
}

#[test]
fn every_pattern_compiles() {
    // Sources that are regexes must compile; the block scanner entry is prose plus a regex.
    for p in redact::patterns() {
        regex::Regex::new(p.source).unwrap_or_else(|e| panic!("{}: {e}", p.name));
    }
}

/// Fragments that sit right on the boundaries of the patterns.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("Authorization".to_owned()),
        Just("Bearer ".to_owned()),
        Just("token".to_owned()),
        Just("id-token".to_owned()),
        Just("password".to_owned()),
        Just("client-key-data".to_owned()),
        Just("data".to_owned()),
        Just("stringData".to_owned()),
        Just("eyJabcde.fghijk.lmnop".to_owned()),
        Just("-----BEGIN PRIVATE KEY-----".to_owned()),
        Just("-----END PRIVATE KEY-----".to_owned()),
        Just("[redacted]".to_owned()),
        Just("Some(".to_owned()),
        Just("http://u:p@h".to_owned()),
        Just("://".to_owned()),
        Just("@".to_owned()),
        Just("ApiKey ".to_owned()),
        Just("a\"b".to_owned()),
        Just("ByteString([1, 2])".to_owned()),
        prop::sample::select(vec![
            ": ", "=", " ", "\n", "\r\n", "\"", "'", "\\\"", "{", "}", "[", "]", "(", ")", ",",
            "  ", "\t",
        ])
        .prop_map(str::to_owned),
        "[A-Za-z0-9._=/+-]{0,12}",
    ]
}

proptest! {
    #[test]
    fn never_panics_on_arbitrary_utf8(s in any::<String>()) {
        let _ = redact(&s);
    }

    #[test]
    fn never_panics_and_is_idempotent_near_patterns(parts in prop::collection::vec(fragment(), 0..14)) {
        let input: String = parts.concat();
        let once = redact(&input).into_owned();
        let twice = redact(&once).into_owned();
        prop_assert_eq!(&twice, &once, "input: {:?}", input);
    }
}
