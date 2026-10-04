//! TLS and proxy behaviour of the pool: assertions on the built kube `Config`, plus real
//! client builds (which open no connection) to prove rustls and kube accept the settings.

use std::io::Write;
use std::sync::Arc;

use base64::Engine as _;
use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{ErrorKind, OxiError};
use parking_lot::Mutex;
use tracing_subscriber::fmt::MakeWriter;

use super::*;
use crate::kubeconfig::{Strictness, load_kubeconfig_from_paths_blocking};

const CA_PEM: &str = include_str!("../../tests/fixtures/tls/ca.pem");
const TOKEN: &str = "tok-SECRET-token-value";

fn b64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

fn kubeconfig(cluster_extra: &str) -> Kubeconfig {
    kubeconfig_at("10.1.2.3", cluster_extra)
}

fn kubeconfig_at(host: &str, cluster_extra: &str) -> Kubeconfig {
    let yaml = format!(
        r#"
apiVersion: v1
kind: Config
current-context: t
clusters:
- name: c
  cluster:
    server: https://{host}:6443
{cluster_extra}
users:
- name: u
  user:
    token: {TOKEN}
contexts:
- name: t
  context: {{cluster: c, user: u}}
"#
    );
    Kubeconfig::from_yaml(&yaml).expect("test kubeconfig parses")
}

fn context() -> ContextName {
    ContextName::from("t")
}

fn definition(cluster_extra: &str) -> ContextDefinition {
    ContextDefinition::from_kubeconfig(&kubeconfig(cluster_extra), &context()).expect("context t")
}

fn config_with_env(cluster_extra: &str, env: Option<&str>) -> Result<kube::Config, OxiError> {
    let env = ProxyEnv::with_https_proxy(env.map(str::to_owned));
    build_config(&definition(cluster_extra), &PoolConfig::default(), &env)
}

fn config_of(cluster_extra: &str) -> Result<kube::Config, OxiError> {
    config_with_env(cluster_extra, None)
}

/// Builds a client the way the pool does (sync; needs a runtime for kube's tower buffer).
fn build(cluster_extra: &str, env: Option<&str>) -> Result<(), OxiError> {
    let factory = KubeClientFactory::new(ProxyEnv::with_https_proxy(env.map(str::to_owned)));
    factory
        .build(&definition(cluster_extra), &PoolConfig::default())
        .map(drop)
}

fn ca_data_line(pem: &str) -> String {
    format!("    certificate-authority-data: {}", b64(pem))
}

fn write_ca(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(CA_PEM.as_bytes()))
        .expect("write CA");
    path
}

// --- insecure-skip-tls-verify ------------------------------------------------------

#[test]
fn insecure_skip_tls_verify_sets_accept_invalid_certs_and_the_flag() {
    let extra = "    insecure-skip-tls-verify: true";
    let config = config_of(extra).expect("config");
    assert!(config.accept_invalid_certs);
    assert!(definition(extra).tls_verification_disabled());
}

#[test]
fn verification_stays_on_unless_the_kubeconfig_says_otherwise() {
    for extra in [
        "",
        "    insecure-skip-tls-verify: false",
        "    tls-server-name: kubernetes.internal",
        "    proxy-url: http://proxy:3128",
    ] {
        let config = config_of(extra).expect("config");
        assert!(!config.accept_invalid_certs, "{extra:?}");
        assert!(!definition(extra).tls_verification_disabled(), "{extra:?}");
    }
    let pool = ClientPool::new(kubeconfig(""), PoolConfig::default());
    assert!(!pool.tls_verification_disabled(&context()));
    assert!(!pool.tls_verification_disabled(&ContextName::from("unknown")));
}

#[test]
fn pool_reports_the_insecure_flag_for_a_context() {
    let pool = ClientPool::new(
        kubeconfig("    insecure-skip-tls-verify: true"),
        PoolConfig::default(),
    );
    assert!(pool.tls_verification_disabled(&context()));
}

/// Tracing output per emitting thread. A process-wide subscriber (installed once) keeps the
/// callsite interest stable; a scoped `with_default` per test races with other tests that hit
/// the same callsite while no subscriber is set and loses events.
static LOGS: Mutex<Vec<(std::thread::ThreadId, Vec<u8>)>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
struct PerThreadLogs;

impl std::io::Write for PerThreadLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let id = std::thread::current().id();
        let mut logs = LOGS.lock();
        match logs.iter_mut().find(|(t, _)| *t == id) {
            Some((_, bytes)) => bytes.extend_from_slice(buf),
            None => logs.push((id, buf.to_vec())),
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for PerThreadLogs {
    type Writer = PerThreadLogs;
    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

fn install_log_capture() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(PerThreadLogs)
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("no other global subscriber");
    });
}

/// Runs `run` on this thread and returns what it logged.
fn logs_of(run: impl FnOnce()) -> String {
    install_log_capture();
    let id = std::thread::current().id();
    LOGS.lock().retain(|(t, _)| *t != id);
    run();
    let bytes = LOGS
        .lock()
        .iter()
        .find(|(t, _)| *t == id)
        .map(|(_, b)| b.clone())
        .unwrap_or_default();
    String::from_utf8(bytes).expect("utf-8 logs")
}

#[test]
fn insecure_tls_logs_one_secret_free_warning() {
    let extra = format!(
        "    insecure-skip-tls-verify: true\n{}\n    proxy-url: http://user:proxy-pass@proxy:3128",
        ca_data_line(CA_PEM)
    );
    let logs = logs_of(|| {
        config_of(&extra).expect("config");
    });
    assert_eq!(
        logs.matches("verification is disabled").count(),
        1,
        "{logs}"
    );
    assert!(logs.contains("WARN"), "{logs}");
    assert!(logs.contains("10.1.2.3"), "names the host: {logs}");
    assert!(logs.contains("context=t"), "names the context: {logs}");
    for secret in [TOKEN, "proxy-pass", &b64(CA_PEM)[..20], "BEGIN CERTIFICATE"] {
        assert!(!logs.contains(secret), "leaked {secret:?}: {logs}");
    }
}

#[test]
fn verified_connections_log_no_tls_warning() {
    let logs = logs_of(|| {
        config_of(&ca_data_line(CA_PEM)).expect("config");
        config_of("").expect("config");
    });
    assert!(!logs.contains("verification is disabled"), "{logs}");
}

// --- certificate authorities ---------------------------------------------------------

#[tokio::test]
async fn inline_ca_data_loads_and_builds() {
    let extra = ca_data_line(CA_PEM);
    let config = config_of(&extra).expect("config");
    assert_eq!(config.root_cert.as_ref().map(Vec::len), Some(1));
    assert_eq!(config.root_cert_file, None, "inline data is not reloadable");
    build(&extra, None).expect("client builds with the CA");
}

#[tokio::test]
async fn absolute_ca_file_loads_and_is_registered_for_reload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_ca(&dir, "ca.pem");
    let extra = format!("    certificate-authority: {}", path.display());
    let config = config_of(&extra).expect("config");
    assert_eq!(config.root_cert.as_ref().map(Vec::len), Some(1));
    assert_eq!(config.root_cert_file.as_deref(), Some(path.as_path()));
    build(&extra, None).expect("client builds with the reloading CA file");
}

#[tokio::test]
async fn relative_ca_file_is_resolved_by_the_kubeconfig_loader() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ca = write_ca(&dir, "ca.pem");
    let file = dir.path().join("config");
    let yaml = serde_yaml_like(&kubeconfig("    certificate-authority: ca.pem"));
    std::fs::write(&file, yaml).expect("write kubeconfig");

    let loaded = load_kubeconfig_from_paths_blocking(&[file], Strictness::default()).expect("load");
    let def = ContextDefinition::from_kubeconfig(&loaded.merged, &context()).expect("context");
    let config = build_config(&def, &PoolConfig::default(), &ProxyEnv::default()).expect("config");
    assert_eq!(config.root_cert.as_ref().map(Vec::len), Some(1));
    let reload = config.root_cert_file.expect("registered for reload");
    assert!(reload.is_absolute());
    assert_eq!(
        reload.canonicalize().expect("canonical"),
        ca.canonicalize().expect("canonical")
    );
    KubeClientFactory::new(ProxyEnv::default())
        .build(&def, &PoolConfig::default())
        .expect("client builds");
}

/// The kubeconfig as YAML, with the relative CA path kept (serde round trip of the model).
fn serde_yaml_like(config: &Kubeconfig) -> String {
    serde_json::to_string(config).expect("kubeconfig serialises (JSON is valid YAML)")
}

#[test]
fn inline_ca_data_wins_over_the_file_and_is_not_reloaded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_ca(&dir, "ca.pem");
    let extra = format!(
        "{}\n    certificate-authority: {}",
        ca_data_line(CA_PEM),
        path.display()
    );
    let config = config_of(&extra).expect("config");
    assert_eq!(config.root_cert.as_ref().map(Vec::len), Some(1));
    assert_eq!(config.root_cert_file, None);
}

#[test]
fn insecure_mode_does_not_register_a_reloading_ca() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_ca(&dir, "ca.pem");
    let extra = format!(
        "    insecure-skip-tls-verify: true\n    certificate-authority: {}",
        path.display()
    );
    assert_eq!(config_of(&extra).expect("config").root_cert_file, None);
}

#[test]
fn empty_ca_fields_count_as_unset() {
    let extra = "    certificate-authority-data: \"\"\n    certificate-authority: \"\"";
    let config = config_of(extra).expect("config");
    assert_eq!(
        config.root_cert, None,
        "system roots, not an empty trust store"
    );
    assert_eq!(config.root_cert_file, None);
}

#[test]
fn ca_without_any_pem_certificate_is_a_validation_error() {
    let err = config_of(&ca_data_line("this is not a certificate\n")).expect_err("no PEM");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("no PEM certificates"), "{err}");
    assert!(err.message().contains("certificate authority"), "{err}");
    let key_only = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
    let err = config_of(&ca_data_line(key_only)).expect_err("a key is not a CA");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn malformed_ca_base64_and_missing_file_are_validation_errors_without_the_data() {
    let err = config_of("    certificate-authority-data: \"c2VjcmV0!S0VZ\"").expect_err("bad b64");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("certificate authority"), "{err}");
    let text = format!("{err} {err:?}");
    assert!(
        !text.contains("c2VjcmV0") && !text.contains("S0VZ"),
        "{text}"
    );

    let err = config_of("    certificate-authority: /nonexistent/ca.pem").expect_err("no file");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("certificate authority"), "{err}");
}

#[tokio::test]
async fn pem_with_undecodable_certificate_body_fails_the_build_as_validation() {
    let bad = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
    let extra = ca_data_line(bad);
    let err = build(&extra, None).expect_err("rustls rejects the certificate");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("certificate authority"), "{err}");
}

// --- tls-server-name -------------------------------------------------------------------

#[tokio::test]
async fn tls_server_name_reaches_the_config_and_builds() {
    let extra = "    tls-server-name: kubernetes.internal";
    let config = config_of(extra).expect("config");
    assert_eq!(
        config.tls_server_name.as_deref(),
        Some("kubernetes.internal")
    );
    assert_eq!(config.cluster_url.to_string(), "https://10.1.2.3:6443/");
    build(extra, None).expect("client builds");
}

#[tokio::test]
async fn invalid_tls_server_name_is_a_validation_error() {
    let err = build("    tls-server-name: \"not a host name!\"", None).expect_err("bad name");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("TLS server name"), "{err}");
}

// --- proxies -------------------------------------------------------------------------

#[tokio::test]
async fn every_supported_proxy_scheme_is_kept_and_builds() {
    for (url, expect) in [
        ("http://proxy.corp:3128", "http://proxy.corp:3128/"),
        ("https://proxy.corp:3129", "https://proxy.corp:3129/"),
        ("socks5://proxy.corp:1080", "socks5://proxy.corp:1080/"),
        ("socks5h://proxy.corp:1080", "socks5://proxy.corp:1080/"),
        ("HTTP://proxy.corp:3128", "http://proxy.corp:3128/"),
        (
            "http://user:pw@proxy.corp:3128",
            "http://user:pw@proxy.corp:3128/",
        ),
    ] {
        let extra = format!("    proxy-url: \"{url}\"");
        let config = config_of(&extra).expect("config");
        assert_eq!(
            config.proxy_url.map(|u| u.to_string()).as_deref(),
            Some(expect),
            "{url}"
        );
        build(&extra, None).unwrap_or_else(|e| panic!("{url}: {e}"));
    }
}

#[test]
fn kubeconfig_proxy_beats_the_environment_for_every_scheme() {
    for url in ["http://k:1", "https://k:1", "socks5://k:1"] {
        let extra = format!("    proxy-url: {url}");
        let config = config_with_env(&extra, Some("http://env:2")).expect("config");
        assert_eq!(
            config.proxy_url.map(|u| u.to_string()),
            Some(format!("{url}/"))
        );
    }
    let config = config_with_env("", Some("socks5://env:2")).expect("config");
    assert_eq!(
        config.proxy_url.map(|u| u.to_string()).as_deref(),
        Some("socks5://env:2/")
    );
}

#[test]
fn an_unsupported_proxy_scheme_is_an_unsupported_error_naming_the_scheme_only() {
    let err = config_of("    proxy-url: ftp://user:secret-pw@proxy:21").expect_err("ftp");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(err.message().contains("`ftp`"), "{err}");
    assert!(err.message().contains("http, https or socks5"), "{err}");
    let text = format!("{err} {err:?}");
    assert!(
        !text.contains("secret-pw") && !text.contains("proxy:21"),
        "{text}"
    );
}

#[test]
fn the_same_rules_apply_to_the_environment_proxy() {
    let err = config_with_env("", Some("gopher://env:70")).expect_err("gopher");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[test]
fn a_proxy_without_a_scheme_is_a_validation_error() {
    let err = config_of("    proxy-url: proxy.corp:3128").expect_err("no scheme");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("scheme"), "{err}");
}

#[test]
fn defaults_have_no_tls_or_proxy_surprises() {
    let config = config_of("").expect("config");
    assert!(!config.accept_invalid_certs);
    assert_eq!(config.root_cert, None);
    assert_eq!(config.root_cert_file, None);
    assert_eq!(config.tls_server_name, None);
    assert_eq!(config.proxy_url, None);
}

#[test]
fn empty_ca_data_next_to_an_absolute_ca_file_still_reloads_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_ca(&dir, "ca.pem");
    let extra = format!(
        "    certificate-authority-data: \"\"\n    certificate-authority: {}",
        path.display()
    );
    let config = config_of(&extra).expect("config");
    assert_eq!(config.root_cert.as_ref().map(Vec::len), Some(1));
    assert_eq!(config.root_cert_file.as_deref(), Some(path.as_path()));
}

#[tokio::test]
async fn the_pool_build_path_emits_the_insecure_warning() {
    install_log_capture();
    // The build runs on a blocking-pool thread, so look across threads for a unique host.
    let host = "10.77.77.77";
    let pool = ClientPool::with_parts(
        kubeconfig_at(host, "    insecure-skip-tls-verify: true"),
        PoolConfig::default(),
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    pool.get(&context()).await.expect("client");

    let logs: String = LOGS
        .lock()
        .iter()
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .collect();
    let line = logs
        .lines()
        .find(|l| l.contains(host))
        .unwrap_or_else(|| panic!("no warning for {host}: {logs}"));
    assert!(
        line.contains("WARN") && line.contains("verification is disabled"),
        "{line}"
    );
    assert!(!line.contains(TOKEN), "{line}");
}
