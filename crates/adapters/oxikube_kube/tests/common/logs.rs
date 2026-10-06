//! Fixtures for the log scenarios (E04-S08): pods that write predictable output, and helpers
//! to read a stream with a deadline.

use std::time::Duration;

use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::log::LogLine;
use oxikube_kube::KubeLogs;
use oxikube_ports::LogStream;
use oxikube_testkit::images;
use serde_json::json;

/// The busybox image of the log scenarios; `cargo xtask kind-up` pulls it into every node.
pub const BUSYBOX: &str = images::E2E_BUSYBOX;

/// The adapter under test on `client`.
pub fn logs(client: &Client) -> KubeLogs {
    KubeLogs::new(client.clone())
}

/// One container: its name and the shell script it runs.
pub struct Script<'a> {
    pub name: &'a str,
    pub script: &'a str,
}

/// A pod running each `containers` script with `sh -c`, with an `emptyDir` at `/data` in every
/// container (state that survives a container restart) and a one second grace period.
pub fn script_pod(
    name: &str,
    labels: &[(&str, &str)],
    restart_policy: &str,
    containers: &[Script<'_>],
) -> Pod {
    let labels: serde_json::Map<_, _> = labels
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": name, "labels": labels},
        "spec": {
            "restartPolicy": restart_policy,
            "terminationGracePeriodSeconds": 1,
            "volumes": [{"name": "data", "emptyDir": {}}],
            "containers": containers.iter().map(|c| json!({
                "name": c.name,
                "image": BUSYBOX,
                "command": ["sh", "-c", c.script],
                "volumeMounts": [{"name": "data", "mountPath": "/data"}],
            })).collect::<Vec<_>>(),
        },
    }))
    .expect("pod json")
}

/// Creates `pod` in `namespace`.
pub async fn create(client: &Client, namespace: &str, pod: &Pod) {
    Api::<Pod>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), pod)
        .await
        .expect("create pod");
}

/// Waits until every container of `name` is running or has run (so there is a log to read); an
/// image pull does not count against the deadline and a failure carries the pod's events
/// (see [`super::pods`]).
pub async fn wait_started(client: &Client, namespace: &str, name: &str) {
    super::pods::wait_started(client, namespace, name).await;
}

/// Reads `count` lines from `stream`, failing the test when `within` passes first or an error
/// item or the end of the stream comes before that.
pub async fn read_lines(stream: &mut LogStream, count: usize, within: Duration) -> Vec<LogLine> {
    let mut lines = Vec::with_capacity(count);
    let read = async {
        while lines.len() < count {
            match stream.next().await {
                Some(Ok(line)) => lines.push(line),
                Some(Err(err)) => panic!("log stream error after {} lines: {err}", lines.len()),
                None => panic!("log stream ended after {} of {count} lines", lines.len()),
            }
        }
    };
    tokio::time::timeout(within, read)
        .await
        .unwrap_or_else(|_| panic!("only {} of {count} lines within {within:?}", lines.len()));
    lines
}

/// The `n` in each `<prefix> <n>` line, panicking on a line of another shape.
pub fn numbers(lines: &[LogLine], prefix: &str) -> Vec<u64> {
    lines
        .iter()
        .map(|l| {
            l.text
                .strip_prefix(prefix)
                .and_then(|rest| rest.trim().parse().ok())
                .unwrap_or_else(|| panic!("unexpected line `{}`", l.text))
        })
        .collect()
}
