//! The node shell: the pod it builds, and its lifecycle (cleanup on exit, error, drop and
//! abort) against a scripted pod API and the testkit's fake exec port.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{ExecOptions, ExitStatus};
use oxikube_testkit::fakes::{ExecScript, ExecStreamCall, FakeExecStreamPort};

use super::fakes::{Call, FakePods};
use crate::remote::exec::node_shell::{
    self, NODE_ANNOTATION, NODE_SHELL_LABEL, NodeShellConfig, node_shell_manifest,
};
use crate::remote::exec::pods::PodStamp;
use crate::remote::exec::wait::Container;

const POD: &str = "oxikube-node-shell-abc12";

fn config() -> NodeShellConfig {
    NodeShellConfig {
        namespace: "debug".into(),
        ..NodeShellConfig::default()
    }
}

#[test]
fn the_manifest_is_a_privileged_host_pod_pinned_to_the_node() {
    let config = NodeShellConfig {
        image: "registry.local/tools:1".into(),
        image_pull_secret: Some("regcred".into()),
        max_lifetime: Duration::from_secs(600),
        ..config()
    };
    let pod = node_shell_manifest("worker-1", &config).expect("manifest");
    assert_eq!(pod["metadata"]["generateName"], "oxikube-node-shell-");
    assert_eq!(pod["metadata"]["namespace"], "debug");
    assert_eq!(pod["metadata"]["labels"][NODE_SHELL_LABEL], "true");
    assert_eq!(pod["metadata"]["annotations"][NODE_ANNOTATION], "worker-1");
    let spec = &pod["spec"];
    assert_eq!(spec["nodeName"], "worker-1");
    assert_eq!(spec["hostPID"], true);
    assert_eq!(spec["hostNetwork"], true);
    assert_eq!(spec["restartPolicy"], "Never");
    assert_eq!(spec["activeDeadlineSeconds"], 600);
    assert_eq!(spec["tolerations"][0]["operator"], "Exists");
    assert_eq!(spec["imagePullSecrets"][0]["name"], "regcred");
    let container = &spec["containers"][0];
    assert_eq!(container["name"], "shell");
    assert_eq!(container["image"], "registry.local/tools:1");
    assert_eq!(container["securityContext"]["privileged"], true);
    assert_eq!(container["command"], serde_json::json!(["sleep", "600"]));
}

#[test]
fn the_default_manifest_has_no_pull_secret_and_a_default_image() {
    let pod = node_shell_manifest("n", &NodeShellConfig::default()).expect("manifest");
    assert!(pod["spec"].get("imagePullSecrets").is_none());
    assert_eq!(
        pod["spec"]["containers"][0]["image"],
        node_shell::DEFAULT_IMAGE
    );
    assert_eq!(pod["metadata"]["namespace"], node_shell::DEFAULT_NAMESPACE);
}

#[test]
fn a_bad_node_namespace_or_image_is_refused() {
    let bad_image = NodeShellConfig {
        image: " ".into(),
        ..NodeShellConfig::default()
    };
    let bad_namespace = NodeShellConfig {
        namespace: "a/b".into(),
        ..NodeShellConfig::default()
    };
    for (node, config) in [
        ("a/b", NodeShellConfig::default()),
        ("", NodeShellConfig::default()),
        ("n", bad_image),
        ("n", bad_namespace),
    ] {
        let err = node_shell_manifest(node, &config).expect_err("refused");
        assert_eq!(err.kind(), ErrorKind::Validation);
    }
}

async fn open(
    exec: &FakeExecStreamPort,
    pods: &Arc<FakePods>,
) -> Result<node_shell::NodeShellSession, OxiError> {
    node_shell::open(exec, pods.clone(), "worker-1", &config()).await
}

#[tokio::test]
async fn open_creates_waits_then_execs_a_tty_shell_in_the_node_namespaces() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script()
        .exec
        .push(Ok(ExecScript::new().stdout("# ").exit_when_told()));

    let mut shell = open(&exec, &pods).await.expect("open");
    assert_eq!(
        (shell.namespace.as_str(), shell.pod.as_str()),
        ("debug", POD)
    );
    let out = shell.session.stdout.as_mut().expect("stdout").next().await;
    assert_eq!(out.expect("chunk").expect("ok"), b"# ");

    let calls = pods.calls();
    assert!(matches!(&calls[0], Call::Create { namespace, .. } if namespace == "debug"));
    assert_eq!(
        calls[1],
        Call::Wait {
            pod: POD.into(),
            container: Container::Regular("shell".into())
        }
    );
    assert_eq!(calls.len(), 2, "nothing is deleted while the shell runs");

    let ExecStreamCall::Exec {
        namespace,
        pod,
        command,
        options,
    } = &exec.recorded_calls()[0]
    else {
        panic!("expected an exec");
    };
    assert_eq!((namespace.as_str(), pod.as_str()), ("debug", POD));
    assert_eq!(&command[..3], ["nsenter", "-t", "1"]);
    assert!(command.iter().any(|a| a == "--"));
    assert_eq!(*options, ExecOptions::interactive().container("shell"));
}

#[tokio::test]
async fn the_pod_is_deleted_when_the_shell_exits() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script()
        .exec
        .push(Ok(ExecScript::new().exit(Ok(ExitStatus::with_code(7)))));
    let shell = open(&exec, &pods).await.expect("open");
    assert!(pods.deletions().is_empty());
    let status = shell.session.status.await.expect("status");
    assert_eq!(
        status.code,
        Some(7),
        "the shell's own result is passed through"
    );
    assert_eq!(
        pods.deletions(),
        [POD],
        "deleted before the status resolved"
    );
}

#[tokio::test]
async fn the_pod_is_deleted_when_the_session_is_dropped() {
    let (pods, mut deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script()
        .exec
        .push(Ok(ExecScript::new().exit_when_told()));
    let shell = open(&exec, &pods).await.expect("open");
    drop(shell);
    let name = tokio::time::timeout(Duration::from_secs(5), deleted.recv())
        .await
        .expect("the cleanup runs in the background")
        .expect("a deletion");
    assert_eq!(name, POD);
}

#[tokio::test]
async fn the_pod_is_deleted_when_the_status_is_dropped_and_the_streams_are_kept() {
    let (pods, mut deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script()
        .exec
        .push(Ok(ExecScript::new().exit_when_told()));
    let mut shell = open(&exec, &pods).await.expect("open");
    let stdout = shell.session.stdout.take();
    drop(shell);
    assert_eq!(deleted.recv().await.as_deref(), Some(POD));
    drop(stdout);
}

#[tokio::test]
async fn dropping_the_status_while_the_delete_is_in_flight_still_deletes_the_pod() {
    let (pods, mut deleted) = FakePods::new();
    pods.stall_next_delete
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script().exec.push(Ok(ExecScript::new()));
    let shell = open(&exec, &pods).await.expect("open");
    // The shell has exited, so the status is inside the stalled delete when the timeout
    // drops it.
    let waited = tokio::time::timeout(Duration::from_millis(200), shell.session.status).await;
    assert!(waited.is_err(), "the delete is still in flight");
    assert_eq!(pods.deletions(), [POD], "the first delete was sent");
    let name = tokio::time::timeout(Duration::from_secs(5), deleted.recv())
        .await
        .expect("the guard deletes in the background")
        .expect("a deletion");
    assert_eq!(name, POD);
    assert_eq!(pods.deletions().len(), 2, "the cancelled delete was redone");
}

#[tokio::test]
async fn cancelling_open_while_its_failure_cleanup_is_in_flight_still_deletes_the_pod() {
    let (pods, mut deleted) = FakePods::new();
    *pods.wait_error.lock() = Some((ErrorKind::Conflict, "ImagePullBackOff"));
    pods.stall_next_delete
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    let opened = tokio::time::timeout(Duration::from_millis(200), open(&exec, &pods)).await;
    assert!(opened.is_err(), "the cleanup is still in flight");
    let name = tokio::time::timeout(Duration::from_secs(5), deleted.recv())
        .await
        .expect("the guard deletes in the background")
        .expect("a deletion");
    assert_eq!(name, POD);
}

#[tokio::test]
async fn a_pod_that_cannot_start_is_deleted_and_the_error_returned() {
    let (pods, _deleted) = FakePods::new();
    *pods.wait_error.lock() = Some((ErrorKind::Conflict, "ImagePullBackOff"));
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    let err = open(&exec, &pods).await.expect_err("fails");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(pods.deletions(), [POD]);
    assert!(exec.recorded_calls().is_empty(), "no exec was attempted");
}

#[tokio::test]
async fn a_failed_exec_deletes_the_pod() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    // Nothing scripted: the fake exec port fails the call.
    let exec = FakeExecStreamPort::new();
    open(&exec, &pods).await.expect_err("exec fails");
    assert_eq!(pods.deletions(), [POD]);
}

#[tokio::test]
async fn a_failed_delete_is_survivable() {
    let (pods, _deleted) = FakePods::new();
    *pods.delete_error.lock() = Some((ErrorKind::Network, "down"));
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script().exec.push(Ok(ExecScript::new()));
    let shell = open(&exec, &pods).await.expect("open");
    shell
        .session
        .status
        .await
        .expect("the shell's status is unaffected");
    assert_eq!(pods.deletions(), [POD]);
}

#[tokio::test]
async fn nothing_is_created_for_an_invalid_request() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    node_shell::open(&exec, pods.clone(), "a/b", &config())
        .await
        .expect_err("invalid node");
    assert!(pods.calls().is_empty());
}

#[tokio::test]
async fn a_custom_shell_replaces_the_default_login_shell() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script().exec.push(Ok(ExecScript::new()));
    let config = NodeShellConfig {
        shell: vec!["zsh".into()],
        ..config()
    };
    node_shell::open(&exec, pods, "worker-1", &config)
        .await
        .expect("open");
    let ExecStreamCall::Exec { command, .. } = &exec.recorded_calls()[0] else {
        panic!("expected an exec");
    };
    assert_eq!(command.last().map(String::as_str), Some("zsh"));
}

#[tokio::test]
async fn the_sweep_deletes_only_old_labelled_leftovers() {
    let (pods, _deleted) = FakePods::new();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs(),
    )
    .expect("fits");
    *pods.listed.lock() = vec![
        PodStamp {
            name: "old".into(),
            created: now - 3600,
        },
        PodStamp {
            name: "fresh".into(),
            created: now - 5,
        },
    ];
    let deleted = node_shell::sweep(&pods, "debug", Duration::from_secs(300))
        .await
        .expect("sweep");
    assert_eq!(deleted, 1);
    assert_eq!(pods.deletions(), ["old"]);
    assert!(matches!(&pods.calls()[0], Call::List { selector } if selector == NODE_SHELL_LABEL));
}
