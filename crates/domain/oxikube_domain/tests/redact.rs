//! Behaviour of `oxikube_domain::redact`: every pattern, idempotence, false positives, panics.

use oxikube_domain::redact::{
    self, MARKER, Redacted, SENSITIVE_FIELDS, SENSITIVE_SUFFIXES, is_sensitive_field, redact,
};
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
fn authorization_with_quoted_parameters() {
    let out = assert_scrubbed(
        r#"Authorization: Digest username="bob", realm="r", nonce="abc123", response="deadbeefcafe" next"#,
        &["abc123", "deadbeefcafe", "bob"],
    );
    assert!(out.ends_with(" next"), "{out}");
    assert_scrubbed(
        r#"Authorization: OAuth oauth_consumer_key="k", oauth_signature="sig sig,}x""#,
        &["sig sig", "x\""],
    );
    let out = assert_scrubbed(r#"Authorization: Basic a, b=c"d tail"#, &["\"d"]);
    assert!(out.ends_with(" tail"), "{out}");
    // JSON-escaped, including a twice-escaped quote inside the value.
    assert_scrubbed(
        r#"{"message":"Authorization: Digest nonce=\"abc123\", response=\"dead\\\"beef\""}"#,
        &["abc123", "dead", "beef"],
    );
    assert_scrubbed("Authorization: Bearer [redacted], nonce=\"zzz\"", &["zzz"]);
}

#[test]
fn authorization_keys_with_a_snake_case_prefix() {
    // `_` is a word character: `\bauthorization` alone never fires inside these keys.
    assert_scrubbed(
        "proxy_authorization: Basic YWRtaW46c2VjcmV0UFc=",
        &["YWRtaW46c2VjcmV0UFc="],
    );
    assert_scrubbed("x_authorization=SECRETXAUTH", &["SECRETXAUTH"]);
    #[derive(Debug)]
    #[allow(dead_code)]
    struct Upstream {
        proxy_authorization: Option<String>,
        x_authorization: String,
    }
    let upstream = Upstream {
        proxy_authorization: Some("Basic YWRtaW46c2VjcmV0UFc=".into()),
        x_authorization: "SECRETXAUTH".into(),
    };
    let out = assert_scrubbed(
        &format!("{upstream:?}"),
        &["YWRtaW46c2VjcmV0UFc=", "SECRETXAUTH"],
    );
    assert_eq!(
        out,
        r#"Upstream { proxy_authorization: Some("[redacted]"), x_authorization: "[redacted]" }"#
    );
    assert_scrubbed(
        &format!("{upstream:#?}"),
        &["YWRtaW46c2VjcmV0UFc=", "SECRETXAUTH"],
    );
}

#[test]
fn closers_inside_a_bare_value_are_part_of_the_secret() {
    for (input, secrets) in [
        ("password=p4ss;w0rd!", &["p4ss", "w0rd"][..]),
        ("password=abc,def", &["abc", "def"]),
        ("token=abc)xyz", &["abc", "xyz"]),
        ("api_key=k]e}y", &["k]e}y", "e}y"]),
        ("data: {tls.key: abc;def}", &["abc", "def"]),
    ] {
        assert_scrubbed(input, secrets);
    }
    // A closer that ends the value keeps the framing around it.
    for (input, expected) in [
        ("(token=abc)", "(token=[redacted])"),
        ("{token: abc}", "{token: [redacted]}"),
        ("[token=abc], next", "[token=[redacted]], next"),
        ("token=abc, user=bob", "token=[redacted], user=bob"),
        ("token=abc; user=bob", "token=[redacted]; user=bob"),
        (
            r#"{"token":abc,"user":"bob"}"#,
            r#"{"token":[redacted],"user":"bob"}"#,
        ),
        ("data: {tls.key: abc;def}", "data: {tls.key: [redacted]}"),
        ("data: {a: b,c: d}", "data: {a: [redacted],c: [redacted]}"),
        ("data: {a: x),c: y}", "data: {a: [redacted]),c: [redacted]}"),
        (
            r#"secret_data={"user": "SECRETV"}"#,
            r#"secret_data={"user": "[redacted]"}"#,
        ),
    ] {
        assert_eq!(redact(input), expected, "{input}");
    }
}

#[test]
fn restored_error_text_coverage() {
    // Shapes the S04 `auth::scrub` stand-in redacted before this module replaced it.
    for (input, secrets) in [
        (
            "error: Basic dXNlcjpwYXNzd29yZA== rejected",
            &["dXNlcjpwYXNzd29yZA=="][..],
        ),
        ("Basic dXNlcjpwYXNz failed", &["dXNlcjpwYXNz"]),
        ("api_key=SECRETAPIKEY123", &["SECRETAPIKEY123"]),
        ("x-api-key: SECRETAPIKEY123", &["SECRETAPIKEY123"]),
        (r#"{"apikey":"FAKEVALUE"}"#, &["FAKEVALUE"]),
        ("error: api_key=FAKEVALUE", &["FAKEVALUE"]),
        ("secret: SECRETVAL", &["SECRETVAL"]),
        ("client_secret=SECRETVAL", &["SECRETVAL"]),
        ("Bearer SHORT1 rejected", &["SHORT1"]),
        (
            "Authorization: Bearer FAKE-abc123 rejected",
            &["FAKE-abc123"],
        ),
    ] {
        let out = assert_scrubbed(input, secrets);
        assert!(!out.is_empty(), "{input}");
    }
    let out = redact("error: Basic dXNlcjpwYXNzd29yZA== rejected");
    assert_eq!(out, "error: Basic [redacted] rejected");
    // Prose after the scheme words stays.
    for text in [
        "basic auth is disabled for this cluster",
        "Bearer Token authentication failed",
        "secretName: db-creds; kind: Secret",
    ] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn secret_data_blocks_behind_escaped_newlines() {
    let yaml = "apiVersion: v1\nkind: Secret\ndata:\n  tls.key: TLSKEYVALUE\n  user: YWRtaW4=\ntype: Opaque\n";
    // A JSON log line: the manifest is one string with `\n` escapes.
    let json_line = serde_json::json!({ "fields": { "manifest": yaml } }).to_string();
    let out = assert_scrubbed(&json_line, &["TLSKEYVALUE", "YWRtaW4="]);
    assert!(out.contains(r"\ntype: Opaque\n"), "{out}");
    serde_json::from_str::<serde_json::Value>(&out).expect("still valid JSON");
    // The same manifest as the message (header right after the JSON framing).
    let json_line = serde_json::json!({ "message": "data:\n  tls.key: TLSKEYVALUE" }).to_string();
    assert_scrubbed(&json_line, &["TLSKEYVALUE"]);
    // Escaped twice: a `Debug`-printed `&str` field inside a JSON log line.
    let json_line = serde_json::json!({ "manifest": format!("{yaml:?}") }).to_string();
    assert!(json_line.contains(r"data:\\n"), "{json_line}");
    assert_scrubbed(&json_line, &["TLSKEYVALUE", "YWRtaW4="]);
    // `Debug` of a `&str` field, as a text log line prints it; and `\r\n` escapes.
    assert_scrubbed(&format!("manifest={yaml:?}"), &["TLSKEYVALUE", "YWRtaW4="]);
    assert_scrubbed(
        &format!("{:?}", yaml.replace('\n', "\r\n")),
        &["TLSKEYVALUE", "YWRtaW4="],
    );
}

#[test]
fn an_escaped_newline_inside_a_quoted_block_value_does_not_end_the_block() {
    // A double-quoted YAML value holding a `\n` escape (a PEM under stringData), then an
    // entry whose name is not sensitive on its own.
    let yaml = "stringData:\n  config: \"a\\nb\"\n  tls.key: SECRETVALUE1\n";
    assert_eq!(
        redact(yaml),
        "stringData:\n  config: \"[redacted]\"\n  tls.key: [redacted]\n"
    );
    assert_eq!(
        redact("data:\n  ca.crt: \"x\\ny\"\n  tls.key: S\ntype: Opaque"),
        "data:\n  ca.crt: \"[redacted]\"\n  tls.key: [redacted]\ntype: Opaque"
    );
    // The same manifest one escape level down (a JSON string or a `Debug`-quoted `&str`),
    // and two (a `Debug`-quoted `&str` inside a JSON string).
    let message = format!("manifest:\n{yaml}");
    let json_line = serde_json::json!({ "message": message, "target": "app" }).to_string();
    let debug_in_json = serde_json::json!({ "manifest": format!("{message:?}") }).to_string();
    for text in [
        json_line.clone(),
        format!("manifest={message:?} next=1"),
        debug_in_json.clone(),
    ] {
        let out = assert_scrubbed(&text, &["SECRETVALUE1", "b\\"]);
        assert!(out.contains("tls.key: [redacted]"), "{out}");
    }
    for line in [json_line, debug_in_json] {
        let out = redact(&line);
        serde_json::from_str::<serde_json::Value>(&out).expect("still valid JSON");
    }
    // `stringData` is also a secret field name: its escaped line break is not its value, so
    // the header survives for the block scanner. A value after the break is still redacted.
    assert_eq!(
        redact(r#"{"m":"stringData:\n  k: v"}"#),
        r#"{"m":"stringData:\n  k: [redacted]"}"#
    );
    assert_eq!(
        redact(r#""stringData:\\n  k: v""#),
        r#""stringData:\\n  k: [redacted]""#
    );
    assert_eq!(
        redact(r"password:\nhunter2 next"),
        r"password:\n[redacted] next"
    );
}

#[test]
fn a_data_block_ends_with_the_string_that_holds_it() {
    // No trailing newline: the block's last entry is followed by the JSON framing, which must
    // not be read as more block entries.
    let line = serde_json::json!({
        "fields": { "message": "data:\n  tls.key: TLSKEY" },
        "target": "app::sync",
        "line_number": 42,
        "span": { "request_id": 7, "name": "req" },
        "spans": [{ "request_id": 7, "name": "req" }],
    })
    .to_string();
    let out = assert_scrubbed(&line, &["TLSKEY"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    assert_eq!(parsed["fields"]["message"], "data:\n  tls.key: [redacted]");
    assert_eq!(parsed["target"], "app::sync");
    assert_eq!(parsed["line_number"], 42);
    assert_eq!(parsed["span"]["request_id"], 7);
    assert_eq!(parsed["spans"][0]["name"], "req");
    // Escaped twice: the `Debug` string closes first, then the JSON one.
    let line = serde_json::json!({
        "fields": { "manifest": format!("{:?}", "data:\n  tls.key: TLSKEY") },
        "target": "app::sync",
    })
    .to_string();
    let out = assert_scrubbed(&line, &["TLSKEY"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    assert_eq!(parsed["target"], "app::sync");
    // A text log line: the `Debug`-quoted field closes the block, the next field survives.
    let out = assert_scrubbed(
        &format!("manifest={:?} attempt=3", "data:\n  tls.key: TLSKEY"),
        &["TLSKEY"],
    );
    assert!(out.ends_with(r#"tls.key: [redacted]" attempt=3"#), "{out}");
    // A later string in the same record can open its own block.
    let line = serde_json::json!({
        "a": "data:\n  k: ONE",
        "b": "stringData:\n  k: TWO",
        "target": "t",
    })
    .to_string();
    let out = assert_scrubbed(&line, &["ONE", "TWO"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    assert_eq!(parsed["target"], "t");
}

#[test]
fn a_data_block_on_the_first_line_of_an_indented_string() {
    // Pretty JSON: the header follows `    "manifest": "`, the entries are indented from the
    // string's text, which is shallower than the outer line.
    let manifest = "data:\n  tls.key: PRETTYSECRET\n  user: YWRtaW4=\n";
    let pretty = serde_json::to_string_pretty(&serde_json::json!({
        "spec": { "manifest": manifest, "replicas": 2 },
        "target": "app::sync",
    }))
    .expect("serialise");
    let out = assert_scrubbed(&pretty, &["PRETTYSECRET", "YWRtaW4="]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    assert_eq!(
        parsed["spec"]["manifest"],
        "data:\n  tls.key: [redacted]\n  user: [redacted]\n"
    );
    assert_eq!(parsed["spec"]["replicas"], 2);
    assert_eq!(parsed["target"], "app::sync");
    // A second level: the pretty JSON quoted again as a JSON string.
    let twice = serde_json::json!({ "body": pretty }).to_string();
    assert_scrubbed(&twice, &["PRETTYSECRET", "YWRtaW4="]);

    // `{:#?}` Debug of a struct holding the manifest, and of one nested deeper.
    #[derive(Debug)]
    #[allow(dead_code)]
    struct Spec {
        manifest: &'static str,
        replicas: u32,
    }
    #[derive(Debug)]
    #[allow(dead_code)]
    struct Wrapper {
        spec: Spec,
    }
    let spec = Spec {
        manifest: "data:\n  tls.key: DBGPRETTY\n",
        replicas: 2,
    };
    let out = assert_scrubbed(&format!("{spec:#?}"), &["DBGPRETTY"]);
    assert!(out.contains(r#"tls.key: [redacted]\n","#), "{out}");
    assert!(out.contains("replicas: 2"), "{out}");
    let out = assert_scrubbed(&format!("{:#?}", Wrapper { spec }), &["DBGPRETTY"]);
    assert!(out.contains("replicas: 2"), "{out}");
    // A manifest that starts indented keeps its entries deeper than the header.
    let spec = Spec {
        manifest: "  data:\n    tls.key: DBGPRETTY\n  type: Opaque\n",
        replicas: 2,
    };
    let out = assert_scrubbed(&format!("{spec:#?}"), &["DBGPRETTY"]);
    assert!(out.contains(r"\n  type: Opaque\n"), "{out}");
}

#[test]
fn every_sensitive_field_name_is_caught_as_text() {
    // The JSON log layer has no name-aware visitor: every name `is_sensitive_field` accepts
    // must be caught by the text patterns in every form the formatters print.
    let mut names: Vec<String> = SENSITIVE_FIELDS.iter().map(|n| (*n).to_owned()).collect();
    names.push("stringData".to_owned());
    for suffix in SENSITIVE_SUFFIXES {
        names.push((*suffix).to_owned());
        names.push(format!("x_{suffix}"));
        names.push(format!("http.{suffix}"));
    }
    for name in &names {
        assert!(is_sensitive_field(name), "{name}");
        for form in [
            format!(r#"{{"{name}":"SECRETV"}}"#),
            format!(r#"{{"{name}": "SECRETV"}}"#),
            format!(r#"{{"{name}":"{{\"user\": \"SECRETV\"}}"}}"#),
            format!(r#"{{\"{name}\":\"SECRETV\"}}"#),
            format!("{name}=SECRETV"),
            format!("{name}: SECRETV"),
            format!(r#"{name}="SECRETV""#),
            format!(r#"{name}={{"user": "SECRETV"}}"#),
            format!(r#"S {{ {name}: Some("SECRETV") }}"#),
        ] {
            assert_scrubbed(&form, &["SECRETV"]);
        }
    }
}

#[test]
fn braces_and_quotes_inside_data_values() {
    assert_scrubbed(
        r#"data: {"k": "a}b-secret", "j": "other"}"#,
        &["a}b", "b-secret", "other"],
    );
    assert_scrubbed(
        r#"{"data":"{\"k\": \"x\\\"y-secret\", \"j\": \"z-secret\"}"}"#,
        &["y-secret", "z-secret"],
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
    // A scheme word read as prose after another scheme word is still a scheme.
    assert_eq!(
        redact("using basic Bearer s3cr3tT0k3nXYZ now"),
        "using basic Bearer [redacted] now"
    );
    assert_eq!(redact("Bearer Bearer +"), "Bearer Bearer [redacted]");
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
    // Pretty `{:#?}` puts an `Option`'s value on the line after `Some(`.
    let out = assert_scrubbed(
        "Config {\n    token: Some(\n        \"abcdef123456\",\n    ),\n}",
        &["abcdef123456"],
    );
    assert!(
        out.contains("token: Some(\n        \"[redacted]\""),
        "{out}"
    );
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
fn secret_data_block_scalar_lines_are_redacted_whole() {
    // The issue's example: only the `|` indicator used to be replaced.
    let out = assert_scrubbed(
        "stringData:\n  config.yaml: |\n    password-ish line1\n    line2\n  j: v\n",
        &["password-ish", "line1", "line2", ": v\n"],
    );
    assert_eq!(
        out,
        "stringData:\n  config.yaml: [redacted]\n    [redacted]\n    [redacted]\n  j: [redacted]\n"
    );

    // Every indicator, a blank line inside the scalar, CRLF, and what follows the block.
    for indicator in ["|", "|-", "|+", ">", ">-", "|2"] {
        let manifest = format!(
            "data:\r\n  a: {indicator}\r\n    first\r\n\r\n      deeper: x\r\n  b: v\r\ntype: Opaque\r\n"
        );
        let out = assert_scrubbed(&manifest, &["first", "deeper", "x\r", ": v\r"]);
        assert!(out.contains("\r\n\r\n"), "blank line kept: {out:?}");
        assert!(out.contains("      [redacted]\r\n"), "indent kept: {out:?}");
        assert!(out.ends_with("type: Opaque\r\n"), "{out:?}");
    }

    // A PEM written as a block scalar (BEGIN line included), then a plain multi-line value.
    let manifest = "stringData:\n  tls.key: |\n    PEMLINE1\n    PEMLINE2\n  note: first\n    second\n    third\nkind: Secret\n";
    let out = assert_scrubbed(manifest, &["PEMLINE", "first", "second", "third"]);
    assert!(out.ends_with("kind: Secret\n"), "{out}");

    // A multi-line double-quoted scalar keeps its continuation out of the output too.
    let out = assert_scrubbed(
        "data:\n  q: \"one\n    two\"\n  r: v\n",
        &["one", "two", ": v"],
    );
    assert!(out.contains("  r: [redacted]\n"), "{out}");

    // Entries shallower than the continuation, and non-secret text outside the block, stay.
    let out = assert_scrubbed(
        "metadata:\n  name: db\n    stray: kept\nstringData:\n  a: |\n    s3cret\n",
        &["s3cret"],
    );
    assert!(out.contains("  name: db\n    stray: kept\n"), "{out}");
}

#[test]
fn secret_data_block_scalar_lines_in_escaped_text() {
    let manifest = "kind: Secret\nstringData:\n  config.yaml: |\n    password-ish line1\n    line2\n  j: v\ntype: Opaque\n";
    let secrets = ["password-ish", "line1", "line2", ": v"];

    // JSON log line (`\n`), `Debug`-quoted `&str` (`\n`), both inside one more string (`\\n`).
    let json = serde_json::json!({ "manifest": manifest, "target": "app::sync" }).to_string();
    let out = assert_scrubbed(&json, &secrets);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    assert_eq!(
        parsed["manifest"],
        "kind: Secret\nstringData:\n  config.yaml: [redacted]\n    [redacted]\n    [redacted]\n  j: [redacted]\ntype: Opaque\n"
    );
    assert_eq!(parsed["target"], "app::sync");

    let debug = format!("manifest={manifest:?} target=\"app::sync\"");
    let out = assert_scrubbed(&debug, &secrets);
    assert!(out.contains("target=\"app::sync\""), "{out}");
    assert!(
        out.contains(r"\n    [redacted]\n  j: [redacted]\ntype: Opaque\n"),
        "{out}"
    );

    let twice = serde_json::json!({ "line": json }).to_string();
    assert_scrubbed(&twice, &secrets);
    let twice = serde_json::json!({ "line": debug }).to_string();
    assert_scrubbed(&twice, &secrets);

    // A block scalar whose last line is the end of the quoted string.
    let tail = serde_json::json!({ "m": "data:\n  k: |\n    LASTLINE" }).to_string();
    let out = assert_scrubbed(&tail, &["LASTLINE"]);
    assert!(out.ends_with(r#"    [redacted]"}"#), "{out}");
}

#[test]
fn secret_data_block_scalar_in_pretty_json_string() {
    let manifest = "data:\n  config.yaml: |\n    BLOCKSECRET\n  user: YWRtaW4=\n";
    let pretty = serde_json::to_string_pretty(&serde_json::json!({
        "spec": { "manifest": manifest, "replicas": 2 },
    }))
    .expect("serialise");
    let out = assert_scrubbed(&pretty, &["BLOCKSECRET", "YWRtaW4="]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
    assert_eq!(
        parsed["spec"]["manifest"],
        "data:\n  config.yaml: [redacted]\n    [redacted]\n  user: [redacted]\n"
    );
    assert_eq!(parsed["spec"]["replicas"], 2);
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
        // Shrunk property-test cases: a value after an escaped break, and a scheme word that
        // a later stage redacts.
        "stringData=\\n\"Authorization",
        "data:\\n  a://Bearer Bearer +",
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
        "proxy_authorization",
        "client_secret",
        "x-api-key",
        "openai_api_key",
        "apikey",
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
        Just("proxy_authorization".to_owned()),
        Just("Basic ".to_owned()),
        Just("api_key".to_owned()),
        Just("secret".to_owned()),
        Just("secret_data".to_owned()),
        Just("\\n".to_owned()),
        Just("data:\\n  ".to_owned()),
        Just("stringData:\n  k: \"a\\nb\"".to_owned()),
        Just("\\\\n".to_owned()),
        Just("    \"m\": \"data:\\n".to_owned()),
        Just("stringData:\n  c: |\n    l1\n    l2\n  j: v\n".to_owned()),
        Just("data:\\n  c: >-\\n    l1\\n    l2\\n  j: v".to_owned()),
        Just("\n    l3".to_owned()),
        prop::sample::select(vec![
            ": ", "=", " ", "\n", "\r\n", "\"", "'", "\\\"", "{", "}", "[", "]", "(", ")", ",",
            ";", "  ", "\t",
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
