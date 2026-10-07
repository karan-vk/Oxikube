//! `ClientPool` with a real exec credential plugin (a tiny `sh` script; unix only):
//! the build deadline, and that a plugin outliving it is resumed, not run again.
//! No cluster needed: the plugin runs while the client is built, before any request.
#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kube::config::Kubeconfig;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;
use oxikube_kube::{ClientPool, KubeClientFactory, PoolConfig, ProxyEnv, SystemClock};

const OK_CREDENTIAL: &str = r#"printf '{"apiVersion":"client.authentication.k8s.io/v1","kind":"ExecCredential","status":{"token":"FAKE-PLUGIN-TOKEN","expirationTimestamp":"2099-01-01T00:00:00Z"}}'"#;

/// A kubeconfig whose only user runs `sh <dir>/plugin.sh`; the script appends a line
/// to `<dir>/runs` each time it starts, then sleeps `sleep_secs` and prints a token.
fn pool_with_slow_plugin(dir: &Path, sleep_secs: f32, deadline: Duration) -> ClientPool {
    let script = format!(
        "echo run >> {}\nsleep {sleep_secs}\n{OK_CREDENTIAL}\n",
        dir.join("runs").display()
    );
    let plugin = dir.join("plugin.sh");
    std::fs::write(&plugin, script).unwrap();
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
      args: ["{}"]
      interactiveMode: Never
contexts:
- name: k
  context: {{cluster: k, user: k}}
"#,
        plugin.display()
    );
    let config = PoolConfig {
        exec_deadline: deadline,
        ..PoolConfig::default()
    };
    ClientPool::with_parts(
        Kubeconfig::from_yaml(&yaml).unwrap(),
        config,
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    )
}

fn runs(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("runs"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[tokio::test]
async fn slow_plugin_hits_the_deadline_and_is_resumed_not_rerun() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool_with_slow_plugin(dir.path(), 0.6, Duration::from_millis(150));
    let ctx = ContextName::from("k");

    let started = Instant::now();
    let err = match pool.get(&ctx).await {
        Ok(_) => panic!("the plugin sleeps past the deadline"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::Timeout, "{err:?}");
    assert!(err.is_retryable());
    assert!(err.message().contains("150ms"), "{}", err.message());
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "returned promptly"
    );

    // A retry while the plugin still runs waits on it; no second process starts.
    assert!(pool.get(&ctx).await.is_err());
    assert_eq!(runs(dir.path()), 1);

    // When the build finishes, its client is used. A token plugin runs once per build
    // (the refresh guard takes kube's auth layer over; kube alone runs it three times).
    tokio::time::sleep(Duration::from_millis(800)).await;
    pool.get(&ctx).await.expect("the resumed build succeeds");
    assert_eq!(runs(dir.path()), 1, "one build, no restart");
}
