//! Pool tests against fake kubeconfigs. Servers point at `127.0.0.1:1`: building a
//! kube `Client` opens no connection, so no cluster or network is needed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use kube::Client;
use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use oxikube_domain::{ErrorKind, OxiError};
use parking_lot::Mutex;

use super::*;
use crate::auth::ExecInteractivePolicy;
use crate::kubeconfig::LoadedKubeconfig;

const TOKEN_A: &str = "tok-AAAA-secret-value";
const TOKEN_B: &str = "tok-BBBB-secret-value";
const KEY_DATA: &str = "LS0tLS1CRUdJTiBQUklWQVRFIEtFWS0tLS0tc2VjcmV0LWtleS1kYXRh";
const PASSWORD: &str = "hunter2-password";
const EXEC_ARG: &str = "--secret-exec-arg";

/// Contexts: `a` and `b` with bearer tokens on separate clusters, `c` with basic
/// auth, `d` with an exec plugin that does not exist plus client key data, and
/// `orphan` pointing at a missing cluster.
fn kubeconfig_yaml(server_a: &str, token_a: &str, extra_cluster_a: &str) -> String {
    format!(
        r#"
apiVersion: v1
kind: Config
current-context: a
clusters:
- name: cluster-a
  cluster:
    server: {server_a}
    insecure-skip-tls-verify: true
{extra_cluster_a}
- name: cluster-b
  cluster:
    server: https://127.0.0.1:1
    insecure-skip-tls-verify: true
- name: cluster-c
  cluster:
    server: https://127.0.0.2:1
    insecure-skip-tls-verify: true
users:
- name: user-a
  user:
    token: {token_a}
- name: user-b
  user:
    token: {TOKEN_B}
- name: user-c
  user:
    username: admin
    password: {PASSWORD}
- name: user-d
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      command: /nonexistent/credential-helper
      args: ["{EXEC_ARG}"]
    client-key-data: {KEY_DATA}
contexts:
- name: a
  context: {{cluster: cluster-a, user: user-a}}
- name: b
  context: {{cluster: cluster-b, user: user-b}}
- name: c
  context: {{cluster: cluster-c, user: user-c}}
- name: d
  context: {{cluster: cluster-b, user: user-d}}
- name: orphan
  context: {{cluster: no-such-cluster, user: user-a}}
"#
    )
}

fn base_yaml() -> String {
    kubeconfig_yaml("https://127.0.0.1:1", TOKEN_A, "")
}

fn parse(yaml: &str) -> Kubeconfig {
    Kubeconfig::from_yaml(yaml).expect("test kubeconfig parses")
}

fn ctx(name: &str) -> ContextName {
    ContextName::from(name)
}

/// Wraps the real factory, counting builds; optionally slow or failing first.
struct CountingFactory {
    inner: KubeClientFactory,
    builds: AtomicUsize,
    delay: Duration,
    fail_first: AtomicUsize,
}

impl CountingFactory {
    fn new() -> Arc<Self> {
        Self::with(Duration::ZERO, 0)
    }

    fn with(delay: Duration, fail_first: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: KubeClientFactory::new(ProxyEnv::default()),
            builds: AtomicUsize::new(0),
            delay,
            fail_first: AtomicUsize::new(fail_first),
        })
    }

    fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }
}

impl ClientFactory for CountingFactory {
    fn build(
        &self,
        definition: &ContextDefinition,
        config: &PoolConfig,
    ) -> Result<Client, OxiError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        // Runs on a blocking-pool thread, so sleeping here models a slow exec plugin.
        std::thread::sleep(self.delay);
        let fail = self
            .fail_first
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if fail {
            return Err(OxiError::network("scripted failure"));
        }
        self.inner.build(definition, config)
    }
}

struct FakeClock(Mutex<Instant>);

impl FakeClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Instant::now())))
    }

    fn advance(&self, by: Duration) {
        *self.0.lock() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.0.lock()
    }
}

fn pool_with(
    config: PoolConfig,
    factory: Arc<CountingFactory>,
    clock: Arc<FakeClock>,
) -> ClientPool {
    ClientPool::with_parts(parse(&base_yaml()), config, factory, clock)
}

fn pool(factory: Arc<CountingFactory>) -> ClientPool {
    pool_with(PoolConfig::default(), factory, FakeClock::new())
}

/// `get` that must fail. `Arc<Client>` is not `Debug`, so `expect_err` does not apply.
async fn get_err(pool: &ClientPool, name: &str) -> OxiError {
    match pool.get(&ctx(name)).await {
        Ok(_) => panic!("get({name}) unexpectedly succeeded"),
        Err(err) => err,
    }
}

fn definition(name: &str) -> ContextDefinition {
    ContextDefinition::from_kubeconfig(&parse(&base_yaml()), &ctx(name)).expect("context exists")
}

// --- lazy build and reuse ---------------------------------------------------------

#[tokio::test]
async fn builds_lazily_on_first_get() {
    let factory = CountingFactory::new();
    let pool = pool(factory.clone());
    assert!(pool.is_empty());
    assert_eq!(factory.builds(), 0);

    pool.get(&ctx("a")).await.expect("client");
    assert_eq!(factory.builds(), 1);
    assert!(pool.contains(&ctx("a")));
}

#[tokio::test]
async fn same_context_returns_the_same_arc() {
    let factory = CountingFactory::new();
    let pool = pool(factory.clone());
    let first = pool.get(&ctx("a")).await.expect("client");
    let second = pool.get(&ctx("a")).await.expect("client");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(factory.builds(), 1);
}

#[tokio::test]
async fn two_contexts_get_separate_clients() {
    let factory = CountingFactory::new();
    let pool = pool(factory.clone());
    let a = pool.get(&ctx("a")).await.expect("client a");
    let b = pool.get(&ctx("b")).await.expect("client b");
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(factory.builds(), 2);
    assert_eq!(pool.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_gets_build_once() {
    let factory = CountingFactory::with(Duration::from_millis(50), 0);
    let pool = Arc::new(pool(factory.clone()));
    let tasks: Vec<_> = (0..16)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { pool.get(&ctx("a")).await })
        })
        .collect();
    let mut clients = Vec::new();
    for task in tasks {
        clients.push(task.await.expect("task").expect("client"));
    }
    assert_eq!(factory.builds(), 1);
    assert!(clients.iter().all(|c| Arc::ptr_eq(c, &clients[0])));
}

#[tokio::test]
async fn failed_build_is_not_cached() {
    let factory = CountingFactory::with(Duration::ZERO, 1);
    let pool = pool(factory.clone());
    let err = get_err(&pool, "a").await;
    assert_eq!(err.kind(), ErrorKind::Network);
    pool.get(&ctx("a")).await.expect("second attempt builds");
    assert_eq!(factory.builds(), 2);
}

#[tokio::test]
async fn real_factory_builds_every_auth_shape_without_network() {
    let pool = ClientPool::with_parts(
        parse(&base_yaml()),
        PoolConfig::default(),
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    for name in ["a", "b", "c"] {
        pool.get(&ctx(name))
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

// --- errors -----------------------------------------------------------------------

#[tokio::test]
async fn unknown_context_is_not_found_and_builds_nothing() {
    let factory = CountingFactory::new();
    let pool = pool(factory.clone());
    let err = get_err(&pool, "nope").await;
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(factory.builds(), 0);
    assert!(pool.is_empty());
}

#[tokio::test]
async fn context_with_missing_cluster_is_a_validation_error() {
    let pool = pool(CountingFactory::new());
    let err = get_err(&pool, "orphan").await;
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn failing_exec_plugin_is_an_auth_error_without_plugin_details() {
    let pool = pool(CountingFactory::new());
    let err = get_err(&pool, "d").await;
    assert_eq!(err.kind(), ErrorKind::Auth);
    let text = format!("{err} {err:?}");
    assert!(!text.contains(EXEC_ARG), "{text}");
    assert!(!text.contains("credential-helper"), "{text}");
}

#[tokio::test]
async fn build_over_the_exec_deadline_times_out_and_is_not_cached() {
    let factory = CountingFactory::with(Duration::from_millis(300), 0);
    let config = PoolConfig {
        exec_deadline: Duration::from_millis(50),
        ..PoolConfig::default()
    };
    let pool = pool_with(config, factory.clone(), FakeClock::new());
    let err = get_err(&pool, "a").await;
    assert_eq!(err.kind(), ErrorKind::Timeout);
    assert!(err.is_retryable());
    assert!(err.message().contains("context `a`"), "{err}");
    // The entry stays empty, so the next `get` builds again.
    assert_eq!(factory.builds(), 1);
    let _ = get_err(&pool, "a").await;
    assert_eq!(factory.builds(), 2);
}

#[tokio::test]
async fn plugin_requiring_interaction_is_rejected_by_the_default_policy() {
    let yaml = r#"
apiVersion: v1
kind: Config
clusters:
- name: k
  cluster: {server: "https://127.0.0.1:1", insecure-skip-tls-verify: true}
users:
- name: k
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      command: /nonexistent/sso-login
      args: ["--secret-exec-arg"]
      interactiveMode: Always
contexts:
- name: k
  context: {cluster: k, user: k}
"#;
    let pool = ClientPool::with_parts(
        parse(yaml),
        PoolConfig::default(),
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    let err = get_err(&pool, "k").await;
    assert_eq!(err.kind(), ErrorKind::Auth);
    assert!(!err.is_retryable(), "a person has to sign in");
    assert!(err.message().contains("interactive"), "{err}");
    assert!(!format!("{err:?}").contains(EXEC_ARG));
}

#[tokio::test]
async fn from_loaded_uses_the_merged_kubeconfig() {
    let loaded = LoadedKubeconfig {
        merged: parse(&base_yaml()),
        sources: Vec::new(),
        origins: Default::default(),
        diagnostics: Vec::new(),
    };
    let pool = ClientPool::from_loaded(&loaded, PoolConfig::default());
    pool.get(&ctx("b"))
        .await
        .expect("b from the merged kubeconfig");
    assert_eq!(get_err(&pool, "nope").await.kind(), ErrorKind::NotFound);
}

// --- invalidation -----------------------------------------------------------------

#[tokio::test]
async fn invalidate_drops_the_entry_and_the_next_get_rebuilds() {
    let factory = CountingFactory::new();
    let pool = pool(factory.clone());
    let old = pool.get(&ctx("a")).await.expect("client");
    assert!(pool.invalidate(&ctx("a")));
    assert!(!pool.invalidate(&ctx("a")));
    let new = pool.get(&ctx("a")).await.expect("client");
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(factory.builds(), 2);
}

#[tokio::test]
async fn new_kubeconfig_drops_only_changed_contexts() {
    let factory = CountingFactory::new();
    let pool = pool(factory.clone());
    let a = pool.get(&ctx("a")).await.expect("a");
    let b = pool.get(&ctx("b")).await.expect("b");
    let c = pool.get(&ctx("c")).await.expect("c");

    // Server of cluster-a changes; b and c are untouched.
    let dropped =
        pool.replace_kubeconfig(parse(&kubeconfig_yaml("https://127.0.0.9:1", TOKEN_A, "")));
    assert_eq!(dropped, vec![ctx("a")]);

    assert!(Arc::ptr_eq(&b, &pool.get(&ctx("b")).await.expect("b")));
    assert!(Arc::ptr_eq(&c, &pool.get(&ctx("c")).await.expect("c")));
    assert!(!Arc::ptr_eq(&a, &pool.get(&ctx("a")).await.expect("a")));
    assert_eq!(factory.builds(), 4);
}

#[tokio::test]
async fn rotated_token_and_cluster_settings_count_as_changes() {
    let pool = pool(CountingFactory::new());
    pool.get(&ctx("a")).await.expect("a");
    let dropped = pool.replace_kubeconfig(parse(&kubeconfig_yaml(
        "https://127.0.0.1:1",
        "tok-rotated",
        "",
    )));
    assert_eq!(dropped, vec![ctx("a")]);

    pool.get(&ctx("a")).await.expect("a");
    let proxied = kubeconfig_yaml(
        "https://127.0.0.1:1",
        "tok-rotated",
        "    proxy-url: http://proxy.example:3128",
    );
    assert_eq!(pool.replace_kubeconfig(parse(&proxied)), vec![ctx("a")]);
}

#[tokio::test]
async fn identical_kubeconfig_keeps_every_client() {
    let pool = pool(CountingFactory::new());
    pool.get(&ctx("a")).await.expect("a");
    pool.get(&ctx("b")).await.expect("b");
    assert!(pool.replace_kubeconfig(parse(&base_yaml())).is_empty());
    assert_eq!(pool.len(), 2);
}

#[tokio::test]
async fn removed_context_is_dropped_and_then_not_found() {
    let pool = pool(CountingFactory::new());
    pool.get(&ctx("b")).await.expect("b");
    let without_b = base_yaml().replace(
        "- name: b\n  context: {cluster: cluster-b, user: user-b}\n",
        "",
    );
    assert_eq!(pool.replace_kubeconfig(parse(&without_b)), vec![ctx("b")]);
    let err = get_err(&pool, "b").await;
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

// --- eviction ---------------------------------------------------------------------

fn idle_policy(max_idle: Duration) -> PoolConfig {
    PoolConfig {
        eviction: EvictionPolicy {
            max_idle: Some(max_idle),
            max_entries: None,
        },
        ..PoolConfig::default()
    }
}

#[tokio::test]
async fn idle_unreferenced_client_is_evicted_after_the_limit() {
    let clock = FakeClock::new();
    let pool = pool_with(
        idle_policy(Duration::from_secs(60)),
        CountingFactory::new(),
        clock.clone(),
    );
    drop(pool.get(&ctx("a")).await.expect("a"));

    clock.advance(Duration::from_secs(59));
    assert!(pool.evict_idle().is_empty());
    clock.advance(Duration::from_secs(1));
    assert_eq!(pool.evict_idle(), vec![ctx("a")]);
    assert!(pool.is_empty());
}

#[tokio::test]
async fn referenced_client_is_never_evicted() {
    let clock = FakeClock::new();
    let pool = pool_with(
        idle_policy(Duration::from_secs(60)),
        CountingFactory::new(),
        clock.clone(),
    );
    let held = pool.get(&ctx("a")).await.expect("a");

    clock.advance(Duration::from_secs(3600));
    assert!(pool.evict_idle().is_empty());
    assert!(Arc::ptr_eq(&held, &pool.get(&ctx("a")).await.expect("a")));

    drop(held);
    clock.advance(Duration::from_secs(3600));
    assert_eq!(pool.evict_idle(), vec![ctx("a")]);
}

#[tokio::test]
async fn idle_time_counts_from_release_not_from_the_last_get() {
    let clock = FakeClock::new();
    let pool = pool_with(
        idle_policy(Duration::from_secs(60)),
        CountingFactory::new(),
        clock.clone(),
    );
    let held = pool.get(&ctx("a")).await.expect("a");

    // Held for 20 minutes, with the periodic sweep seeing it in use.
    clock.advance(Duration::from_secs(20 * 60));
    assert!(pool.evict_idle().is_empty());
    drop(held);

    // Released: not evicted at once, only after a full `max_idle` of idleness.
    assert!(pool.evict_idle().is_empty());
    clock.advance(Duration::from_secs(59));
    assert!(pool.evict_idle().is_empty());
    clock.advance(Duration::from_secs(1));
    assert_eq!(pool.evict_idle(), vec![ctx("a")]);
}

#[tokio::test]
async fn pinned_context_is_never_evicted_until_unpinned() {
    let clock = FakeClock::new();
    let pool = pool_with(
        idle_policy(Duration::from_secs(60)),
        CountingFactory::new(),
        clock.clone(),
    );
    pool.set_pinned(&ctx("a"), true);
    drop(pool.get(&ctx("a")).await.expect("a"));

    clock.advance(Duration::from_secs(3600));
    assert!(pool.evict_idle().is_empty());
    pool.set_pinned(&ctx("a"), false);
    // Unpinning starts the idle clock; the client goes after a full `max_idle`.
    assert!(pool.evict_idle().is_empty());
    clock.advance(Duration::from_secs(60));
    assert_eq!(pool.evict_idle(), vec![ctx("a")]);
}

#[tokio::test]
async fn entry_cap_evicts_least_recently_used_idle_clients_on_get() {
    let clock = FakeClock::new();
    let config = PoolConfig {
        eviction: EvictionPolicy {
            max_idle: None,
            max_entries: Some(2),
        },
        ..PoolConfig::default()
    };
    let pool = pool_with(config, CountingFactory::new(), clock.clone());
    let held_a = pool.get(&ctx("a")).await.expect("a");
    clock.advance(Duration::from_secs(1));
    drop(pool.get(&ctx("b")).await.expect("b"));
    clock.advance(Duration::from_secs(1));
    drop(pool.get(&ctx("c")).await.expect("c"));
    clock.advance(Duration::from_secs(1));

    // The sweep on the next get trims to the cap: `a` is held, so the oldest idle
    // entry (`b`) goes.
    assert_eq!(pool.len(), 2);
    assert!(pool.contains(&ctx("a")));
    assert!(!pool.contains(&ctx("b")));
    assert!(pool.contains(&ctx("c")));
    drop(held_a);
}

// --- Config assertions --------------------------------------------------------------

#[test]
fn built_config_carries_pool_timeouts_retry_and_compression() {
    let config = build_config(
        &definition("a"),
        &PoolConfig::default(),
        &ProxyEnv::default(),
    )
    .expect("config");
    assert_eq!(config.connect_timeout, Some(DEFAULT_CONNECT_TIMEOUT));
    assert_eq!(config.read_timeout, None);
    assert_eq!(config.write_timeout, Some(DEFAULT_WRITE_TIMEOUT));
    assert!(
        config.default_retry,
        "RetryPolicy::server_retry is installed"
    );
    assert!(!config.disable_compression, "gzip stays on");
    assert_eq!(config.cluster_url.to_string(), "https://127.0.0.1:1/");
}

#[test]
fn custom_pool_config_is_applied() {
    let pool_config = PoolConfig {
        connect_timeout: Some(Duration::from_secs(3)),
        read_timeout: Some(Duration::from_secs(3600)),
        write_timeout: None,
        retry: RetryMode::Disabled,
        exec_policy: ExecInteractivePolicy::IfAvailable,
        exec_deadline: Duration::from_secs(5),
        eviction: EvictionPolicy::NEVER,
    };
    let config =
        build_config(&definition("a"), &pool_config, &ProxyEnv::default()).expect("config");
    assert_eq!(config.connect_timeout, Some(Duration::from_secs(3)));
    assert_eq!(config.read_timeout, Some(Duration::from_secs(3600)));
    assert_eq!(config.write_timeout, None);
    assert!(!config.default_retry);
}

#[test]
fn kubeconfig_disable_compression_is_respected() {
    let yaml = kubeconfig_yaml(
        "https://127.0.0.1:1",
        TOKEN_A,
        "    disable-compression: true",
    );
    let def = ContextDefinition::from_kubeconfig(&parse(&yaml), &ctx("a")).expect("a");
    let config = build_config(&def, &PoolConfig::default(), &ProxyEnv::default()).expect("config");
    assert!(config.disable_compression);
}

fn proxy_of(extra_cluster_a: &str, env: Option<&str>) -> Result<Option<String>, OxiError> {
    let yaml = kubeconfig_yaml("https://127.0.0.1:1", TOKEN_A, extra_cluster_a);
    let def = ContextDefinition::from_kubeconfig(&parse(&yaml), &ctx("a")).expect("a");
    let env = ProxyEnv::with_https_proxy(env.map(str::to_owned));
    build_config(&def, &PoolConfig::default(), &env).map(|c| c.proxy_url.map(|u| u.to_string()))
}

#[test]
fn kubeconfig_proxy_url_wins_over_env() {
    let proxy = proxy_of(
        "    proxy-url: http://from-kubeconfig:3128",
        Some("http://from-env:8080"),
    );
    assert_eq!(
        proxy.expect("config").as_deref(),
        Some("http://from-kubeconfig:3128/")
    );
}

#[test]
fn env_proxy_is_the_fallback() {
    let proxy = proxy_of("", Some("http://from-env:8080"));
    assert_eq!(
        proxy.expect("config").as_deref(),
        Some("http://from-env:8080/")
    );
}

#[test]
fn empty_values_mean_no_proxy() {
    assert_eq!(
        proxy_of("    proxy-url: \"\"", Some("")).expect("config"),
        None
    );
    assert_eq!(proxy_of("", None).expect("config"), None);
}

#[test]
fn invalid_proxy_is_a_validation_error_that_hides_the_url() {
    let err = proxy_of("", Some("http://user:pa ss@bad host")).expect_err("invalid proxy");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(!format!("{err:?}").contains("pa ss"));
}

#[test]
fn malformed_kubeconfig_proxy_url_is_an_invalid_proxy_error_without_the_url() {
    // kube parses the cluster `proxy-url` itself (the same path an unparseable process
    // `HTTPS_PROXY` takes), so this exercises the `ParseProxyUrl` mapping.
    let err =
        proxy_of("    proxy-url: \"http://user:pa ss@bad host\"", None).expect_err("invalid proxy");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("proxy URL"), "{err}");
    assert!(err.message().contains("HTTPS_PROXY"), "{err}");
    let text = format!("{err:?}");
    assert!(
        !text.contains("pa ss") && !text.contains("bad host"),
        "{text}"
    );
}

// --- secrets ------------------------------------------------------------------------

fn assert_no_secrets(text: &str) {
    for secret in [
        TOKEN_A,
        TOKEN_B,
        KEY_DATA,
        PASSWORD,
        EXEC_ARG,
        "credential-helper",
    ] {
        assert!(!text.contains(secret), "leaked {secret:?} in {text}");
    }
}

#[tokio::test]
async fn debug_prints_context_and_host_only() {
    let pool = pool(CountingFactory::new());
    for name in ["a", "b", "c"] {
        pool.get(&ctx(name)).await.expect("client");
    }
    let text = format!("{pool:?}");
    assert_no_secrets(&text);
    assert!(
        text.contains("\"a\"") && text.contains("127.0.0.2"),
        "{text}"
    );

    for name in ["a", "c", "d"] {
        let text = format!("{:?}", definition(name));
        assert_no_secrets(&text);
        assert!(text.contains(&format!("\"{name}\"")), "{text}");
    }
}

#[test]
fn proxy_env_debug_hides_the_url() {
    let env = ProxyEnv::with_https_proxy(Some("http://user:proxy-secret@proxy:3128".into()));
    let text = format!("{env:?}");
    assert!(!text.contains("proxy-secret"), "{text}");
}

#[test]
fn definition_slices_only_the_context_its_cluster_and_user() {
    let def = definition("c");
    let kc = def.kubeconfig();
    assert_eq!(kc.current_context.as_deref(), Some("c"));
    assert_eq!(kc.contexts.len(), 1);
    assert_eq!(kc.clusters[0].name, "cluster-c");
    assert_eq!(kc.auth_infos[0].name, "user-c");
    assert_eq!(def.server_host().as_deref(), Some("127.0.0.2"));
}

/// A user with certificate data and malformed key data. `!` is not base64, so the
/// decode error would quote it (and its offset) if the source were attached.
fn malformed_key_kubeconfig() -> Kubeconfig {
    parse(
        r#"
apiVersion: v1
kind: Config
clusters:
- name: k
  cluster: {server: "https://127.0.0.1:1", insecure-skip-tls-verify: true}
users:
- name: k
  user:
    client-certificate-data: LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t
    client-key-data: "c2VjcmV0!S0VZ"
contexts:
- name: k
  context: {cluster: k, user: k}
"#,
    )
}

#[tokio::test]
async fn malformed_client_key_error_carries_no_key_bytes() {
    let pool = ClientPool::with_parts(
        malformed_key_kubeconfig(),
        PoolConfig::default(),
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    let err = get_err(&pool, "k").await;
    assert_eq!(err.kind(), ErrorKind::Validation, "{err:?}");
    assert!(err.message().contains("client key"), "{err}");
    assert!(std::error::Error::source(&err).is_none());
    let text = format!("{err} {err:?}");
    for leak in ["c2VjcmV0", "S0VZ", "Invalid symbol", "offset"] {
        assert!(!text.contains(leak), "leaked {leak:?} in {text}");
    }
}

#[test]
fn malformed_certificate_authority_error_has_no_source() {
    let yaml = kubeconfig_yaml(
        "https://127.0.0.1:1",
        TOKEN_A,
        "    certificate-authority-data: \"bm90!YmFzZTY0\"",
    );
    let def = ContextDefinition::from_kubeconfig(&parse(&yaml), &ctx("a")).expect("a");
    let err =
        build_config(&def, &PoolConfig::default(), &ProxyEnv::default()).expect_err("bad CA data");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(err.message().contains("certificate authority"), "{err}");
    assert!(std::error::Error::source(&err).is_none());
}

#[test]
fn safe_kubeconfig_errors_keep_their_source() {
    let def = definition("orphan");
    let err = build_config(&def, &PoolConfig::default(), &ProxyEnv::default())
        .expect_err("missing cluster");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(std::error::Error::source(&err).is_some());
}
