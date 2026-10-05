//! Fixtures for the exec, attach, node shell and debug scenarios (E04-S09): busybox pods and
//! helpers to read a session's output.

use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use oxikube_ports::exec::OutputStream;
use serde_json::{Value, json};

use super::logs;

/// Small, and has `sh`, `cat`, `stty`, `sleep` and `nsenter`.
pub const BUSYBOX: &str = "busybox:1.37";

/// How long a scenario waits for output it expects.
pub const OUTPUT_DEADLINE: Duration = Duration::from_secs(30);

async fn create(client: &Client, namespace: &str, pod: Value) {
    let pod: Pod = serde_json::from_value(pod).expect("pod");
    logs::create(client, namespace, &pod).await;
}

/// A pod `name` that sleeps, with a container named `main`.
pub async fn create_sleeper(client: &Client, namespace: &str, name: &str) {
    create(
        client,
        namespace,
        json!({
            "metadata": {"name": name},
            "spec": {"terminationGracePeriodSeconds": 1, "containers": [
                {"name": "main", "image": BUSYBOX, "command": ["sleep", "3600"]}]},
        }),
    )
    .await;
}

/// A pod `name` whose main process is `cat` with stdin open, so `attach` talks to it.
pub async fn create_cat(client: &Client, namespace: &str, name: &str) {
    create(
        client,
        namespace,
        json!({
            "metadata": {"name": name},
            "spec": {"terminationGracePeriodSeconds": 1, "containers": [
                {"name": "main", "image": BUSYBOX, "command": ["cat"], "stdin": true}]},
        }),
    )
    .await;
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
