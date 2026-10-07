//! The deadline on credential refreshes inside a live client (E03-F432), with real exec
//! plugins (tiny `sh` scripts; unix only) and a throwaway HTTP server standing in for the API
//! server. No cluster needed.
//!
//! Every plugin hands out an already expired token on its first run (the build), so kube
//! refreshes it on the first request: run 2 is the refresh under test, and it hangs or not as
//! each test decides.
#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kube::config::{ExecConfig, ExecInteractiveMode, Kubeconfig};
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;
use oxikube_kube::auth::{ExecInteractivePolicy, build_client_bounded, classify};
use oxikube_kube::{ClientPool, KubeClientFactory, PoolConfig, ProxyEnv, SystemClock};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

const VERSION: &str = r#"{"major":"1","minor":"30","gitVersion":"v1.30.0","gitCommit":"x","gitTreeState":"clean","buildDate":"2024-01-01T00:00:00Z","goVersion":"go1.22","compiler":"gc","platform":"linux/amd64"}"#;

/// Serves `/version` on a free port and records each request's `Authorization` header.
async fn serve() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let auth = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .starts_with("authorization:")
                            .then(|| l.to_owned())
                    })
                    .map(|l| l.split_once(':').unwrap().1.trim().to_owned())
                    .unwrap_or_default();
                log.lock().push(auth);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{VERSION}",
                    VERSION.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, seen)
}

/// A plugin that records each start in `<dir>/runs`. Run 1 returns an expired `TOKEN-1`
/// at once; later runs sleep `hang_secs`, then return `TOKEN-<n>` valid until 2099.
fn config(dir: &Path, port: u16, hang_secs: f32) -> Config {
    let script = format!(
        r#"echo run >> {runs}
n=$(wc -l < {runs} | tr -d ' ')
if [ "$n" -ge 2 ]; then
  sleep {hang_secs}
  expiry=2099-01-01T00:00:00Z
else
  expiry=2020-01-01T00:00:00Z
fi
printf '{{"apiVersion":"client.authentication.k8s.io/v1","kind":"ExecCredential","status":{{"token":"TOKEN-%s","expirationTimestamp":"%s"}}}}' "$n" "$expiry"
"#,
        runs = dir.join("runs").display()
    );
    let path = dir.join("plugin.sh");
    std::fs::write(&path, script).unwrap();
    let mut config = Config::new(format!("http://127.0.0.1:{port}").parse().unwrap());
    config.auth_info.exec = Some(ExecConfig {
        api_version: Some("client.authentication.k8s.io/v1".into()),
        command: Some("sh".into()),
        args: Some(vec![path.to_string_lossy().into_owned()]),
        interactive_mode: Some(ExecInteractiveMode::Never),
        ..Default::default()
    });
    config
}

async fn build(config: Config, refresh_deadline: Duration) -> Client {
    tokio::task::spawn_blocking(move || {
        build_client_bounded(config, ExecInteractivePolicy::Never, None, refresh_deadline)
    })
    .await
    .expect("build task")
    .expect("the first plugin run succeeds")
}

fn runs(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("runs"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[tokio::test]
async fn a_refresh_inside_the_deadline_sends_the_new_token() {
    let dir = tempfile::tempdir().unwrap();
    let (port, seen) = serve().await;
    let client = build(config(dir.path(), port, 0.0), Duration::from_secs(10)).await;
    assert_eq!(runs(dir.path()), 1, "one run per build, not three");

    client.apiserver_version().await.expect("request");
    assert_eq!(*seen.lock(), ["Bearer TOKEN-2"]);
    assert_eq!(runs(dir.path()), 2);
    client.apiserver_version().await.expect("token is cached");
    assert_eq!(runs(dir.path()), 2, "a fresh token is not refreshed again");
}

#[tokio::test]
async fn a_hung_refresh_fails_at_the_deadline_and_later_requests_do_not_queue() {
    let dir = tempfile::tempdir().unwrap();
    let (port, seen) = serve().await;
    let client = build(config(dir.path(), port, 3.0), Duration::from_millis(300)).await;

    let started = Instant::now();
    let err = client.apiserver_version().await.expect_err("hung refresh");
    assert!(started.elapsed() < Duration::from_millis(1500), "bounded");
    let oxi = classify(&err);
    assert_eq!(oxi.kind(), ErrorKind::Auth, "{err}");
    assert!(oxi.is_retryable(), "a rebuilt client can recover");
    assert!(oxi.message().contains("300ms"), "{}", oxi.message());

    // The plugin still runs: the next request fails at once instead of waiting for it, and
    // starts no second plugin process.
    let started = Instant::now();
    let err = client.apiserver_version().await.expect_err("still stalled");
    assert!(started.elapsed() < Duration::from_millis(100), "fails fast");
    assert_eq!(classify(&err).kind(), ErrorKind::Auth);
    assert_eq!(runs(dir.path()), 2, "no pile-up of plugin processes");
    assert!(seen.lock().is_empty(), "nothing was sent without a token");
}

#[tokio::test]
async fn the_client_recovers_when_the_plugin_finally_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (port, seen) = serve().await;
    let client = build(config(dir.path(), port, 1.0), Duration::from_millis(200)).await;

    client.apiserver_version().await.expect_err("times out");
    tokio::time::sleep(Duration::from_millis(1600)).await;

    client.apiserver_version().await.expect("recovered");
    assert_eq!(
        *seen.lock(),
        ["Bearer TOKEN-2"],
        "the refreshed token is used"
    );
    assert_eq!(
        runs(dir.path()),
        2,
        "the slow run was resumed, not repeated"
    );
}

#[tokio::test]
async fn a_cancelled_request_does_not_start_a_second_plugin_or_fail_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let (port, seen) = serve().await;
    let client = build(config(dir.path(), port, 1.0), Duration::from_secs(30)).await;

    // The caller gives up while the healthy refresh runs; kube's future is dropped, which
    // alone would release the token mutex and let the next request run the plugin again.
    let cancelled =
        tokio::time::timeout(Duration::from_millis(150), client.apiserver_version()).await;
    assert!(cancelled.is_err(), "the request was still waiting");

    // The refresh is still inside its deadline, so this request is not failed (which would
    // make `retry_once` rebuild the client and run a second plugin): it queues on the
    // detached refresh and uses its token.
    client
        .apiserver_version()
        .await
        .expect("queues on the refresh in flight");
    assert_eq!(*seen.lock(), ["Bearer TOKEN-2"]);
    assert_eq!(runs(dir.path()), 2, "one refresh, never repeated");
}

#[tokio::test]
async fn a_cancelled_hung_refresh_stalls_the_client_only_after_the_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let (port, seen) = serve().await;
    let client = build(config(dir.path(), port, 3.0), Duration::from_millis(400)).await;

    let cancelled =
        tokio::time::timeout(Duration::from_millis(100), client.apiserver_version()).await;
    assert!(cancelled.is_err());

    // Inside the deadline of the cancelled refresh: waits, then gives up at its own deadline.
    let started = Instant::now();
    let err = client.apiserver_version().await.expect_err("hung refresh");
    assert!(started.elapsed() >= Duration::from_millis(300), "queued");
    let oxi = classify(&err);
    assert!(oxi.is_retryable(), "{err}");
    assert!(oxi.message().contains("400ms"), "{}", oxi.message());

    // Now the client is stalled: fail fast, no second plugin.
    let started = Instant::now();
    client.apiserver_version().await.expect_err("stalled");
    assert!(started.elapsed() < Duration::from_millis(100), "fails fast");
    assert_eq!(runs(dir.path()), 2);
    assert!(seen.lock().is_empty());
}

/// The app builds clients through `ClientPool`, which passes `PoolConfig::exec_refresh_deadline`
/// to the factory: the configured value, not a default, bounds the refresh.
#[tokio::test]
async fn the_pool_bounds_a_hung_refresh_by_its_configured_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let (port, seen) = serve().await;
    let plugin = config(dir.path(), port, 5.0);
    let exec = plugin.auth_info.exec.clone().unwrap();
    let yaml = format!(
        r#"
apiVersion: v1
kind: Config
clusters:
- name: k
  cluster: {{server: "http://127.0.0.1:{port}"}}
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
        exec.args.unwrap()[0]
    );
    let pool = ClientPool::with_parts(
        Kubeconfig::from_yaml(&yaml).unwrap(),
        PoolConfig {
            exec_refresh_deadline: Duration::from_millis(300),
            ..PoolConfig::default()
        },
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    let client = pool.get(&ContextName::from("k")).await.expect("build");
    assert_eq!(runs(dir.path()), 1, "one plugin run per build");

    let started = Instant::now();
    let err = client.apiserver_version().await.expect_err("hung refresh");
    assert!(started.elapsed() < Duration::from_millis(2000), "bounded");
    let oxi = classify(&err);
    assert_eq!(oxi.kind(), ErrorKind::Auth, "{err}");
    assert!(oxi.is_retryable());
    assert!(oxi.message().contains("300ms"), "{}", oxi.message());
    assert!(seen.lock().is_empty());
}

/// The app exits with `Runtime::shutdown_background` (`oxikube_runtime::GlobalTokio`), which
/// drops every task, including a request that is waiting on a hung refresh: its `Refresh`
/// is dropped while the runtime is going away and must neither panic nor wait for the
/// plugin. (Tokio abandons the blocking thread running the plugin; the guard cannot and
/// does not try to kill it, and a plain `Runtime::drop` would join that thread.)
#[test]
fn a_hung_refresh_does_not_hold_up_runtime_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let client = runtime.block_on(async {
        let (port, _seen) = serve().await;
        build(config(dir.path(), port, 4.0), Duration::from_secs(30)).await
    });
    // Mid-refresh when the runtime goes away: well inside the 30 s deadline.
    let in_flight = runtime.spawn(async move { client.apiserver_version().await });
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(runs(dir.path()), 2, "the refresh plugin is running");

    let started = Instant::now();
    runtime.shutdown_background();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert!(!in_flight.is_finished(), "never completed: it was dropped");
}
