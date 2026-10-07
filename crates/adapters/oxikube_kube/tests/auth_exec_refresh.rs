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

use kube::config::{ExecConfig, ExecInteractiveMode};
use kube::{Client, Config};
use oxikube_domain::ErrorKind;
use oxikube_kube::auth::{ExecInteractivePolicy, build_client_bounded, classify};
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
async fn a_cancelled_request_does_not_start_a_second_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let (port, _seen) = serve().await;
    let client = build(config(dir.path(), port, 1.0), Duration::from_secs(30)).await;

    // The caller gives up while the refresh runs; kube's future is dropped, which alone would
    // release the token mutex and let the next request run the plugin again.
    let cancelled =
        tokio::time::timeout(Duration::from_millis(150), client.apiserver_version()).await;
    assert!(cancelled.is_err(), "the request was still waiting");

    client
        .apiserver_version()
        .await
        .expect_err("refresh in flight");
    assert_eq!(runs(dir.path()), 2);

    tokio::time::sleep(Duration::from_millis(1500)).await;
    client.apiserver_version().await.expect("refresh finished");
    assert_eq!(runs(dir.path()), 2);
}

#[test]
fn a_hung_refresh_does_not_hold_up_runtime_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (port, _seen) = serve().await;
        let client = build(config(dir.path(), port, 4.0), Duration::from_millis(100)).await;
        client.apiserver_version().await.expect_err("hung refresh");
    });

    // What `oxikube_runtime` does when the app exits: the blocking thread running the plugin
    // is abandoned, not joined.
    let started = Instant::now();
    runtime.shutdown_background();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}
