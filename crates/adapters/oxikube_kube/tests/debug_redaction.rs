//! No credential reaches `Debug` output, error messages or trace-level logs.
//!
//! Every struct in this crate that holds (or is built from) a kubeconfig gets the same
//! treatment: a fake token, basic-auth password, client key data, exec-plugin arguments,
//! plugin stderr and a proxy URL with `user:password@` userinfo go in, and none of them may
//! appear in `{:?}` / `{:#?}`, in the error a failed build returns, or in the formatted
//! tracing output captured at `trace` level. Servers point at `127.0.0.1:1`; building a kube
//! `Client` opens no connection.

use std::io;
use std::sync::{Arc, LazyLock, Mutex, Once};

use kube::config::Kubeconfig;
use oxikube_domain::OxiError;
use oxikube_domain::ids::ContextName;
use oxikube_kube::kubeconfig::{Strictness, load_kubeconfig_from_paths_blocking};
use oxikube_kube::{
    ClientPool, ContextDefinition, DiscoveryConfig, KubeClientFactory, KubeDiscovery, PoolConfig,
    ProxyEnv,
};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

// Fake secrets. Nothing here is a real credential.
const TOKEN: &str = "dbg-token-AAAA-0123456789";
const PASSWORD: &str = "dbg-password-hunter2";
const KEY_DATA: &str = "LS0tLS1CRUdJTiBQUklWQVRFIEtFWS0tLS0tZGJnLWtleS1kYXRh";
const CERT_DATA: &str = "LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0tZGJnLWNlcnQtZGF0YQ==";
const EXEC_ARG: &str = "--dbg-exec-secret-arg";
const EXEC_ENV: &str = "dbg-exec-env-secret";
const PROXY_PASS: &str = "dbg-proxy-pass-xyz";
const BAD_PROXY_PASS: &str = "dbg-bad-proxy-pass";
const STDERR_SECRET: &str = "dbg-stderr-secret-token";
const ALL: &[&str] = &[
    TOKEN,
    PASSWORD,
    KEY_DATA,
    CERT_DATA,
    EXEC_ARG,
    EXEC_ENV,
    PROXY_PASS,
    BAD_PROXY_PASS,
    STDERR_SECRET,
];

fn yaml() -> String {
    format!(
        r#"
apiVersion: v1
kind: Config
current-context: token
clusters:
- name: plain
  cluster: {{server: "https://10.1.2.3:6443", insecure-skip-tls-verify: true}}
- name: proxied
  cluster:
    server: https://10.1.2.4:6443
    insecure-skip-tls-verify: true
    proxy-url: http://px-user:{PROXY_PASS}@127.0.0.1:3128
- name: bad-proxy
  cluster:
    server: https://10.1.2.5:6443
    insecure-skip-tls-verify: true
    proxy-url: "http://px-user:{BAD_PROXY_PASS}@[::1"
users:
- name: token-user
  user: {{token: {TOKEN}}}
- name: basic-user
  user: {{username: admin, password: {PASSWORD}}}
- name: cert-user
  user:
    client-certificate-data: {CERT_DATA}
    client-key-data: {KEY_DATA}
- name: missing-plugin-user
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      command: /nonexistent/credential-helper
      args: ["{EXEC_ARG}"]
      env: [{{name: SECRET_ENV, value: {EXEC_ENV}}}]
      interactiveMode: Never
contexts:
- name: token
  context: {{cluster: plain, user: token-user}}
- name: basic
  context: {{cluster: plain, user: basic-user}}
- name: cert
  context: {{cluster: plain, user: cert-user}}
- name: missing-plugin
  context: {{cluster: plain, user: missing-plugin-user}}
- name: proxied
  context: {{cluster: proxied, user: token-user}}
- name: bad-proxy
  context: {{cluster: bad-proxy, user: token-user}}
"#
    )
}

fn kubeconfig() -> Kubeconfig {
    Kubeconfig::from_yaml(&yaml()).expect("test kubeconfig parses")
}

fn ctx(name: &str) -> ContextName {
    ContextName::from(name)
}

/// Collects all formatted tracing output of this test binary.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
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

static CAPTURE: LazyLock<Capture> = LazyLock::new(Capture::default);

/// Installs a global trace-level subscriber once. Global, not per-thread: client builds run on
/// blocking-pool threads, which do not inherit a thread-local default.
fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("trace"))
            .with_ansi(false)
            .with_writer(CAPTURE.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("one global subscriber");
    });
}

fn captured() -> String {
    String::from_utf8_lossy(&CAPTURE.0.lock().unwrap()).into_owned()
}

#[track_caller]
fn assert_no_secrets(what: &str, text: &str) {
    for secret in ALL {
        assert!(!text.contains(secret), "{what} leaks {secret:?}:\n{text}");
    }
}

/// `{:?}` and `{:#?}` of `value`, for both checks.
#[track_caller]
fn assert_debug_clean(what: &str, value: &dyn std::fmt::Debug) {
    assert_no_secrets(what, &format!("{value:?}"));
    assert_no_secrets(what, &format!("{value:#?}"));
}

/// The error text a caller (and a log line) would show, in every format.
#[track_caller]
fn assert_error_clean(what: &str, err: &OxiError) {
    assert_no_secrets(
        what,
        &format!("{err:?} | {err:#?} | {err} | {}", err.message()),
    );
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        assert_no_secrets(what, &format!("{cause:?} | {cause}"));
        source = cause.source();
    }
}

async fn get_err(pool: &ClientPool, context: &str) -> OxiError {
    match pool.get(&ctx(context)).await {
        Ok(_) => panic!("get({context}) unexpectedly succeeded"),
        Err(err) => err,
    }
}

#[test]
fn context_definition_debug_prints_names_and_hosts_only() {
    for name in [
        "token",
        "basic",
        "cert",
        "missing-plugin",
        "proxied",
        "bad-proxy",
    ] {
        let definition = ContextDefinition::from_kubeconfig(&kubeconfig(), &ctx(name)).unwrap();
        assert_debug_clean(name, &definition);
    }
    let shown = format!(
        "{:?}",
        ContextDefinition::from_kubeconfig(&kubeconfig(), &ctx("token")).unwrap()
    );
    assert!(
        shown.contains("10.1.2.3"),
        "host should still be shown: {shown}"
    );
}

#[test]
fn proxy_env_and_factory_debug_never_print_the_url() {
    let url = format!("http://px-user:{PROXY_PASS}@proxy.corp:3128");
    let env = ProxyEnv::with_https_proxy(Some(url));
    assert_debug_clean("ProxyEnv", &env);
    assert_debug_clean("KubeClientFactory", &KubeClientFactory::new(env));
    assert_debug_clean("PoolConfig", &PoolConfig::default());
}

#[test]
fn loaded_kubeconfig_debug_prints_names_and_counts_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, yaml()).unwrap();
    let loaded = load_kubeconfig_from_paths_blocking(&[path], Strictness::Tolerant).unwrap();
    assert!(loaded.has_usable_source());
    assert_debug_clean("LoadedKubeconfig", &loaded);
}

#[tokio::test]
async fn pool_and_entries_debug_after_building_credentialed_clients() {
    init_tracing();
    let pool = ClientPool::new(kubeconfig(), PoolConfig::default());
    for name in ["token", "basic", "proxied"] {
        pool.get(&ctx(name))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
    // Entries are printed by the pool's Debug, which covers `PoolEntry`.
    let shown = format!("{pool:?}");
    assert_no_secrets("ClientPool", &shown);
    assert_no_secrets("ClientPool", &format!("{pool:#?}"));
    assert!(shown.contains("built: true"), "{shown}");

    let client = (*pool.get(&ctx("token")).await.unwrap()).clone();
    let discovery = KubeDiscovery::with_config(client, DiscoveryConfig::default());
    assert_debug_clean("KubeDiscovery", &discovery);
    assert_no_secrets("trace output", &captured());
}

#[tokio::test]
async fn failed_builds_return_and_log_no_credentials() {
    init_tracing();
    let pool = ClientPool::new(kubeconfig(), PoolConfig::default());
    for name in ["missing-plugin", "bad-proxy", "cert"] {
        let err = get_err(&pool, name).await;
        assert_error_clean(name, &err);
        // What a caller would log: every format, at trace level.
        tracing::trace!(context = name, ?err, "pool build failed");
        tracing::trace!(context = name, %err, "pool build failed");
        tracing::error!(context = name, error = %err.message(), "pool build failed");
    }
    assert_debug_clean("ClientPool after failures", &pool);
    let out = captured();
    assert!(out.contains("pool build failed"), "capture is live:\n{out}");
    assert_no_secrets("trace output", &out);
}

#[cfg(unix)]
#[tokio::test]
async fn plugin_stderr_is_scrubbed_in_the_error() {
    init_tracing();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("plugin.sh");
    std::fs::write(
        &script,
        format!(
            "echo 'refresh failed: Authorization: Bearer {STDERR_SECRET} token={STDERR_SECRET}' >&2\nexit 1\n"
        ),
    )
    .unwrap();
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
      args: ["{}", "{EXEC_ARG}"]
      interactiveMode: Never
contexts:
- name: k
  context: {{cluster: k, user: k}}
"#,
        script.display()
    );
    let pool = ClientPool::new(Kubeconfig::from_yaml(&yaml).unwrap(), PoolConfig::default());
    let err = get_err(&pool, "k").await;
    assert_error_clean("plugin stderr", &err);
    // The scrubbed stderr tail is in the message, so the check above is not vacuous.
    assert!(err.message().contains("refresh failed"), "{err:?}");
    assert!(err.message().contains("[redacted]"), "{err:?}");
    tracing::trace!(?err, "plugin failed");
    assert_no_secrets("trace output", &captured());
}
