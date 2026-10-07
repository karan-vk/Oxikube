//! Fixtures for the exec, attach, node shell and debug scenarios (E04-S09): busybox pods and
//! helpers to read a session's output.

use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use oxikube_ports::exec::OutputStream;
use oxikube_testkit::images;
use oxikube_testkit::integration::pods;
use serde_json::Value;

use super::logs;

/// Small, and has `sh`, `cat`, `stty`, `sleep` and `nsenter`.
pub const BUSYBOX: &str = images::BUSYBOX;

/// How long a scenario waits for output it expects.
pub const OUTPUT_DEADLINE: Duration = Duration::from_secs(30);

async fn create(client: &Client, namespace: &str, pod: Value) {
    let pod: Pod = serde_json::from_value(pod).expect("pod");
    logs::create(client, namespace, &pod).await;
}

/// A pod `name` that sleeps, with a container named `main` ([`pods::sleeper`]).
pub async fn create_sleeper(client: &Client, namespace: &str, name: &str) {
    create(client, namespace, pods::sleeper(name)).await;
}

/// A pod `name` whose main process is `cat` with stdin open, so `attach` talks to it
/// ([`pods::cat`]).
pub async fn create_cat(client: &Client, namespace: &str, name: &str) {
    create(client, namespace, pods::cat(name)).await;
}

/// A pod `name` that prints `tick <n>` once a second, for as long as it lives ([`pods::logger`]).
pub async fn create_logger(client: &Client, namespace: &str, name: &str) {
    create(client, namespace, pods::logger(name)).await;
}

/// A pod `name` with no shell and no tools, the stand-in for a distroless image
/// ([`pods::shell_less`]).
pub async fn create_shell_less(client: &Client, namespace: &str, name: &str) {
    create(client, namespace, pods::shell_less(name)).await;
}

/// The restart count of container `container` in pod `name`; 0 before it has a status.
pub async fn restart_count(client: &Client, namespace: &str, name: &str, container: &str) -> i32 {
    let pod = kube::Api::<Pod>::namespaced(client.clone(), namespace)
        .get(name)
        .await
        .expect("get the pod");
    pod.status
        .and_then(|s| s.container_statuses)
        .and_then(|all| all.into_iter().find(|c| c.name == container))
        .map_or(0, |c| c.restart_count)
}

/// Reads `stream` until its output contains `marker`; returns everything read so far.
/// Panics on a read error, on the end of the stream, or after [`OUTPUT_DEADLINE`].
pub async fn read_until(stream: &mut OutputStream, marker: &str) -> String {
    let mut seen = Vec::new();
    let found = tokio::time::timeout(OUTPUT_DEADLINE, async {
        while let Some(chunk) = stream.next().await {
            seen.extend_from_slice(&chunk.expect("a readable stream"));
            if String::from_utf8_lossy(&seen).contains(marker) {
                return true;
            }
        }
        false
    })
    .await;
    let text = String::from_utf8_lossy(&seen).into_owned();
    assert_eq!(found, Ok(true), "never saw {marker:?}; got {text:?}");
    text
}

/// Everything `stream` yields until it ends.
pub async fn read_all(stream: &mut OutputStream) -> Vec<u8> {
    let mut all = Vec::new();
    tokio::time::timeout(OUTPUT_DEADLINE, async {
        while let Some(chunk) = stream.next().await {
            all.extend_from_slice(&chunk.expect("a readable stream"));
        }
    })
    .await
    .expect("the stream ends");
    all
}

/// Reads backend events until the output contains `marker`; returns everything read so far.
/// Panics on a transport error, on an exit or the end of the stream before the marker, or after
/// [`OUTPUT_DEADLINE`].
pub async fn events_until(
    events: &mut futures::stream::BoxStream<'static, oxikube_ports::BackendEvent>,
    marker: &str,
) -> String {
    events_satisfying(events, marker, |text| text.contains(marker)).await
}

/// [`events_until`] for a condition on everything read so far; `what` names it in the failure.
pub async fn events_satisfying(
    events: &mut futures::stream::BoxStream<'static, oxikube_ports::BackendEvent>,
    what: &str,
    done: impl Fn(&str) -> bool,
) -> String {
    use oxikube_ports::BackendEvent;
    let mut seen = Vec::new();
    let found = tokio::time::timeout(OUTPUT_DEADLINE, async {
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => seen.extend_from_slice(&bytes),
                BackendEvent::Error(error) => panic!("transport error: {error}"),
                BackendEvent::Exited(status) => panic!("exited early: {status:?}"),
            }
            if done(&String::from_utf8_lossy(&seen)) {
                return true;
            }
        }
        false
    })
    .await;
    let text = String::from_utf8_lossy(&seen).into_owned();
    assert_eq!(found, Ok(true), "never saw {what:?}; got {text:?}");
    text
}

/// Drains the backend's events; returns the exit status it ended with.
pub async fn events_exit(
    events: &mut futures::stream::BoxStream<'static, oxikube_ports::BackendEvent>,
) -> oxikube_ports::ExitStatus {
    use oxikube_ports::BackendEvent;
    let exit = tokio::time::timeout(OUTPUT_DEADLINE, async {
        let mut exit = None;
        while let Some(event) = events.next().await {
            if let BackendEvent::Exited(status) = event {
                exit = Some(status);
            }
        }
        exit
    })
    .await
    .expect("the stream ends");
    exit.expect("an exit event")
}
