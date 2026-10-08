//! Exec plugins that return a client certificate run a bounded number of times per client
//! build (E03-F550), with real plugins (tiny `sh` scripts; unix only). No cluster needed:
//! the plugin runs while the client is built, before any request.
//!
//! kube alone runs such a plugin three times per build (expiry, TLS identity, auth layer).
//! `build_client_bounded` runs it twice: the first run tells a certificate from a token, the
//! second moves the certificate into the config and clears `exec`.
#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kube::config::{ExecConfig, ExecInteractiveMode, Kubeconfig};
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;
use oxikube_kube::auth::{ExecInteractivePolicy, build_client_bounded};
use oxikube_kube::{ClientPool, KubeClientFactory, PoolConfig, ProxyEnv, SystemClock};

const CERT: &str = include_str!("fixtures/exec-cert/client.crt");
const KEY: &str = include_str!("fixtures/exec-cert/client.key");
const EXPIRY: &str = "2099-01-01T00:00:00Z";

/// Writes `<dir>/plugin.sh`: it appends a line to `<dir>/runs`, sleeps `sleep_secs`, then
/// prints `<dir>/credential.json`.
fn write_plugin(dir: &Path, credential: &serde_json::Value, sleep_secs: f32) -> String {
    std::fs::write(dir.join("credential.json"), credential.to_string()).unwrap();
    let script = format!(
        "echo run >> {runs}\nsleep {sleep_secs}\ncat {cred}\n",
        runs = dir.join("runs").display(),
        cred = dir.join("credential.json").display()
    );
    let path = dir.join("plugin.sh");
    std::fs::write(&path, script).unwrap();
    path.to_string_lossy().into_owned()
}

fn certificate(cert: &str, key: &str) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "client.authentication.k8s.io/v1",
        "kind": "ExecCredential",
        "status": {
            "clientCertificateData": cert,
            "clientKeyData": key,
            "expirationTimestamp": EXPIRY,
        }
    })
}

fn config(script: &str) -> Config {
    let mut config = Config::new("http://127.0.0.1:1".parse().unwrap());
    config.auth_info.exec = Some(ExecConfig {
        api_version: Some("client.authentication.k8s.io/v1".into()),
        command: Some("sh".into()),
        args: Some(vec![script.to_owned()]),
        interactive_mode: Some(ExecInteractiveMode::Never),
        ..Default::default()
    });
    config
}

async fn build(config: Config) -> oxikube_domain::OxiResult<Client> {
    tokio::task::spawn_blocking(move || {
        build_client_bounded(
            config,
            ExecInteractivePolicy::Never,
            None,
            Duration::from_secs(30),
        )
    })
    .await
    .expect("build task")
}

fn runs(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("runs"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// What kube alone does with the same plugin: the baseline the bound is measured against.
async fn kube_only_runs(config: Config, dir: &Path) -> usize {
    tokio::task::spawn_blocking(move || Client::try_from(config))
        .await
        .unwrap()
        .expect("kube builds the client");
    runs(dir)
}

#[tokio::test]
async fn a_certificate_plugin_runs_fewer_times_than_kube_alone_and_never_more_than_three() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_plugin(dir.path(), &certificate(CERT, KEY), 0.0);
    assert_eq!(
        kube_only_runs(config(&script), dir.path()).await,
        3,
        "premise: kube runs a certificate plugin three times"
    );
    std::fs::remove_file(dir.path().join("runs")).unwrap();

    let client = build(config(&script)).await.expect("build");
    let ran = runs(dir.path());
    assert!(
        ran <= 3,
        "at most as often as before the refresh guard: {ran}"
    );
    assert_eq!(ran, 2, "the probe and one run of ours, none from kube");
    assert_eq!(
        client.valid_until().map(|t| t.to_string()).as_deref(),
        Some(EXPIRY),
        "the plugin's expirationTimestamp is still the client's valid_until"
    );
}

#[tokio::test]
async fn a_key_without_a_trailing_newline_still_forms_an_identity() {
    // kube joins key and certificate; a plugin that omits the final newline must not glue
    // `-----END PRIVATE KEY-----` to `-----BEGIN CERTIFICATE-----`.
    let dir = tempfile::tempdir().unwrap();
    let script = write_plugin(
        dir.path(),
        &certificate(CERT.trim_end(), KEY.trim_end()),
        0.0,
    );
    build(config(&script)).await.expect("build");
    assert_eq!(runs(dir.path()), 2);
}

#[tokio::test]
async fn a_certificate_plugin_that_fits_before_the_guard_fits_the_exec_deadline_through_the_pool() {
    // 0.4 s per run: kube alone needs 3 x 0.4 = 1.2 s; with the old probe 4 x 0.4 = 1.6 s,
    // over the 1.4 s deadline. Now 2 x 0.4 = 0.8 s.
    let dir = tempfile::tempdir().unwrap();
    let script = write_plugin(dir.path(), &certificate(CERT, KEY), 0.4);
    let yaml = format!(
        r#"
apiVersion: v1
kind: Config
clusters:
- name: k
  cluster: {{server: "https://127.0.0.1:1", insecure-skip-tls-verify: true}}
users:
- name: k
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      command: sh
      args: ["{script}"]
      interactiveMode: Never
contexts:
- name: k
  context: {{cluster: k, user: k}}
"#
    );
    let pool = ClientPool::with_parts(
        Kubeconfig::from_yaml(&yaml).unwrap(),
        PoolConfig {
            exec_deadline: Duration::from_millis(1400),
            ..PoolConfig::default()
        },
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    let started = Instant::now();
    pool.get(&ContextName::from("k"))
        .await
        .expect("the build fits the deadline");
    assert!(started.elapsed() < Duration::from_millis(1400));
    assert_eq!(runs(dir.path()), 2);
}

#[tokio::test]
async fn a_token_plugin_still_runs_once() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_plugin(
        dir.path(),
        &serde_json::json!({
            "apiVersion": "client.authentication.k8s.io/v1",
            "kind": "ExecCredential",
            "status": {"token": "FAKE-PLUGIN-TOKEN", "expirationTimestamp": EXPIRY},
        }),
        0.0,
    );
    build(config(&script)).await.expect("build");
    assert_eq!(runs(dir.path()), 1);
}

#[tokio::test]
async fn a_plugin_that_fails_on_its_second_run_is_a_retryable_auth_error_without_leaks() {
    // Run 1 returns the certificate; later runs fail like an expired SSO session.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("credential.json"),
        certificate(CERT, KEY).to_string(),
    )
    .unwrap();
    let script = dir.path().join("plugin.sh");
    std::fs::write(
        &script,
        format!(
            "echo run >> {runs}\nn=$(wc -l < {runs} | tr -d ' ')\n\
             if [ \"$n\" -ge 2 ]; then echo 'credentials service unavailable' >&2; exit 1; fi\n\
             cat {cred}\n",
            runs = dir.path().join("runs").display(),
            cred = dir.path().join("credential.json").display()
        ),
    )
    .unwrap();
    let err = build(config(&script.to_string_lossy()))
        .await
        .err()
        .expect("the second run fails");
    assert_eq!(err.kind(), ErrorKind::Auth, "{err}");
    assert!(err.is_retryable());
    assert!(err.message().contains("credentials service unavailable"));
    assert!(
        !format!("{err:?}").contains("plugin.sh"),
        "command line leaked"
    );
    assert!(!format!("{err:?}").contains("PRIVATE KEY"));
}

#[tokio::test]
async fn an_invalid_certificate_is_a_classified_tls_setup_error() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_plugin(
        dir.path(),
        &certificate("not a certificate", "not a key"),
        0.0,
    );
    let err = build(config(&script)).await.err().expect("rejected");
    // Classified like the same bad identity written in a kubeconfig.
    assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    assert_eq!(runs(dir.path()), 2, "no third run from kube");
}
