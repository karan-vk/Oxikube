//! Kind integration: no token reaches a log sink (E03-S09; redaction layers from E03-S08).
//!
//! The whole run happens under the shipped redacting fmt layers of `oxikube_logging`, text
//! and JSON, at TRACE with the `log` bridge on, so kube-client's request spans, its
//! trace-level response-header dumps and anything the HTTP stack logs are all formatted. Two
//! token contexts connect through the pool (a restricted service account's real token and an
//! unknown bearer token), list, and hit a `Forbidden` and an `Auth` failure. Neither token may
//! appear in either output; a token logged on purpose must come out as `[redacted]`.
//!
//! Its own test binary with a single test: the capture is a process-wide subscriber (client
//! builds and blocking work run on other threads), so nothing else may log into it. Needs
//! `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::io;
use std::sync::{Arc, Mutex};

use k8s_openapi::api::core::v1::{Pod, Secret};
use k8s_openapi::api::rbac::v1::PolicyRule;
use kube::Api;
use kube::api::{ListParams, WatchParams};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;
use oxikube_domain::redact::MARKER;
use oxikube_kube::auth::classify;
use oxikube_logging::{redacting_json_layer, redacting_layer};
use oxikube_testkit::integration::TestNamespace;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use common::{DEADLINE, TestServiceAccount, wait_until, whoami};

/// One sink's formatted output.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("sink lock")).into_owned()
    }

    fn len(&self) -> usize {
        self.0.lock().expect("sink lock").len()
    }

    fn text_since(&self, mark: usize) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("sink lock")[mark..]).into_owned()
    }
}

impl io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("sink lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Sink {
    type Writer = Sink;

    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}

/// The text and JSON sinks behind the shipped redacting layers.
struct Capture {
    text: Sink,
    json: Sink,
}

impl Capture {
    /// Installs the process-wide TRACE subscriber with both redacting layers, and the
    /// `log` -> `tracing` bridge so `log`-based crates (rustls) are captured too.
    fn install() -> Self {
        let capture = Self {
            text: Sink::default(),
            json: Sink::default(),
        };
        tracing_subscriber::registry()
            .with(LevelFilter::TRACE)
            .with(redacting_layer(capture.text.clone()))
            .with(redacting_json_layer(capture.json.clone()))
            .try_init()
            .expect("the only subscriber in this test binary");
        capture
    }

    /// Both outputs, labelled, for the per-sink assertions.
    fn outputs(&self) -> [(&'static str, String); 2] {
        [("text", self.text.text()), ("json", self.json.text())]
    }
}

#[track_caller]
fn assert_no_tokens(what: &str, output: &str, tokens: &[(&str, &str)]) {
    for (name, token) in tokens {
        assert!(
            !output.contains(token),
            "{what} log output leaks the {name} token"
        );
    }
}

#[tokio::test]
async fn no_token_reaches_the_logs() {
    let capture = Capture::install();
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into(), "watch".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-log-restricted", vec![pods_only]).await;
    // Opaque and unique, so a hit can only come from this run.
    let bearer = format!("oxi-bearer-{}", uuid::Uuid::new_v4().simple());
    let tokens = [
        ("service account", account.token.as_str()),
        ("bearer", bearer.as_str()),
    ];

    // Two token contexts on the kind cluster, one pool.
    let restricted = ContextName::from("oxi-log-restricted");
    let unknown = ContextName::from("oxi-log-bearer");
    let mut kubeconfig = kind.with_token_context(restricted.as_str(), &account.token);
    let with_bearer = kind.with_token_context(unknown.as_str(), &bearer);
    kubeconfig
        .auth_infos
        .extend(with_bearer.auth_infos.into_iter().skip(1));
    kubeconfig
        .contexts
        .extend(with_bearer.contexts.into_iter().skip(1));
    let pool = kind.pool(kubeconfig);
    let (restricted_client, bearer_client) =
        tokio::join!(pool.get(&restricted), pool.get(&unknown));
    let restricted_client = restricted_client.expect("restricted client");
    let bearer_client = bearer_client.expect("bearer client builds without contacting the server");

    // Connect and list with the service account's token.
    assert_eq!(
        whoami(&restricted_client).await.expect("whoami"),
        account.username
    );
    let pods = Api::<Pod>::namespaced((*restricted_client).clone(), ns.name());
    wait_until("the Role to allow listing pods", DEADLINE, || async {
        pods.list(&ListParams::default()).await.ok()
    })
    .await;
    // A watch: kube-client dumps its response headers at TRACE level.
    drop(
        pods.watch(&WatchParams::default().timeout(1), "0")
            .await
            .expect("watch pods"),
    );

    // Forbidden: secrets are not in the Role.
    let forbidden = Api::<Secret>::namespaced((*restricted_client).clone(), ns.name())
        .list(&ListParams::default())
        .await
        .expect_err("secrets are not allowed");
    let forbidden = classify(&forbidden);
    assert_eq!(forbidden.kind(), ErrorKind::Forbidden, "{forbidden:?}");
    tracing::warn!(context = %restricted, error = %forbidden, "list secrets failed");

    // Auth: the apiserver does not know the bearer token.
    let unauthorized = Api::<Pod>::namespaced((*bearer_client).clone(), ns.name())
        .list(&ListParams::default())
        .await
        .expect_err("an unknown token is rejected");
    let unauthorized = classify(&unauthorized);
    assert_eq!(unauthorized.kind(), ErrorKind::Auth, "{unauthorized:?}");
    tracing::warn!(context = %unknown, error = ?unauthorized, "list pods failed");

    // The capture saw the run at TRACE: kube-client's request span (with the URL) and the
    // watch's trace-level response-header dump. An empty capture would pass the leak check vacuously.
    let secrets_url = format!("/api/v1/namespaces/{}/secrets", ns.name());
    for (what, output) in capture.outputs() {
        assert!(output.contains("requesting"), "{what}: no request events");
        assert!(output.contains(&secrets_url), "{what}: no request span URL");
        assert!(output.contains("headers:"), "{what}: no TRACE events");
        assert!(output.contains("list secrets failed"), "{what}");
        assert!(output.contains("list pods failed"), "{what}");
        assert_no_tokens(what, &output, &tokens);
    }

    // A token logged on purpose, as a field and inside a header line, comes out redacted.
    let (text_mark, json_mark) = (capture.text.len(), capture.json.len());
    for (name, token) in tokens {
        tracing::info!(token = %token, "deliberate {name} token field");
        tracing::info!("deliberate {name} header: Authorization: Bearer {token}");
    }
    for ((what, output), deliberate) in capture.outputs().into_iter().zip([
        capture.text.text_since(text_mark),
        capture.json.text_since(json_mark),
    ]) {
        // Token check first, and counts only in the messages below: if redaction regresses,
        // the captured text holds the real token and must not reach a panic message.
        assert_no_tokens(what, &output, &tokens);
        let lines = deliberate.matches("deliberate").count();
        assert_eq!(lines, 4, "{what}: {lines} deliberate lines, expected 4");
        let markers = deliberate.matches(MARKER).count();
        assert!(
            markers >= 4,
            "{what}: {markers} redaction markers in the deliberate lines, expected >= 4"
        );
    }
}
