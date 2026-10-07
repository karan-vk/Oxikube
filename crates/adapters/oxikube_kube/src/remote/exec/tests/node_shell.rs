//! The node shell: the pod it builds, and its lifecycle (cleanup on exit, error, drop and
//! abort) against a scripted pod API and the testkit's fake exec port.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{ExecOptions, ExitStatus, NODE_SHELL_LABEL, NodeShellSpec};
use oxikube_testkit::fakes::{ExecScript, ExecStreamCall, FakeExecStreamPort};

use super::fakes::{Call, FakePods};
use crate::remote::exec::node_shell;
use crate::remote::exec::pods::PodStamp;
use crate::remote::exec::wait::Container;

const POD: &str = "oxikube-node-shell-abc12";

fn spec() -> NodeShellSpec {
    NodeShellSpec {
        namespace: "debug".into(),
        ..NodeShellSpec::new("worker-1")
    }
}

async fn open(
    exec: &FakeExecStreamPort,
    pods: &Arc<FakePods>,
) -> Result<node_shell::NodeShellSession, OxiError> {
    node_shell::open(exec, pods.clone(), &Arc::default(), &spec()).await
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
    node_shell::open(
        &exec,
        pods.clone(),
        &Arc::default(),
        &NodeShellSpec::new("a/b"),
    )
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
    let spec = NodeShellSpec {
        shell: vec!["zsh".into()],
        ..spec()
    };
    node_shell::open(&exec, pods, &Arc::default(), &spec)
        .await
        .expect("open");
    let ExecStreamCall::Exec { command, .. } = &exec.recorded_calls()[0] else {
        panic!("expected an exec");
    };
    assert_eq!(command.last().map(String::as_str), Some("zsh"));
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs(),
    )
    .expect("fits")
}

fn stamp(name: &str, created: i64, heartbeat: Option<i64>) -> PodStamp {
    PodStamp {
        name: name.into(),
        created,
        heartbeat,
    }
}

#[tokio::test]
async fn the_sweep_deletes_only_old_labelled_leftovers() {
    let (pods, _deleted) = FakePods::new();
    let now = now();
    *pods.listed.lock() = vec![
        stamp("old", now - 3600, None),
        stamp("fresh", now - 5, None),
    ];
    let deleted = node_shell::sweep(&pods, "debug", Duration::from_secs(300))
        .await
        .expect("sweep");
    assert_eq!(deleted, ["old"]);
    assert_eq!(pods.deletions(), ["old"]);
    assert!(matches!(&pods.calls()[0], Call::List { selector } if selector == NODE_SHELL_LABEL));
}

#[tokio::test]
async fn the_sweep_spares_a_long_open_shell_another_window_keeps_stamping() {
    let (pods, _deleted) = FakePods::new();
    let now = now();
    *pods.listed.lock() = vec![
        // Open for hours, stamped a minute ago: someone's live shell.
        stamp("live", now - 4 * 3600, Some(now - 60)),
        // Its owner stopped stamping long ago: a leftover, however recently it stamped once.
        stamp("orphan", now - 4 * 3600, Some(now - 3600)),
        // Never stamped (an owner that died at once) and old.
        stamp("unstamped", now - 4 * 3600, None),
    ];
    let deleted = node_shell::sweep(&pods, "debug", Duration::from_secs(900))
        .await
        .expect("sweep");
    assert_eq!(deleted, ["orphan", "unstamped"]);
    assert_eq!(pods.deletions(), ["orphan", "unstamped"]);
}

#[tokio::test(start_paused = true)]
async fn an_open_shell_stamps_its_pod_alive_until_it_ends() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script()
        .exec
        .push(Ok(ExecScript::new().exit_when_told()));
    let shell = open(&exec, &pods).await.expect("open");
    assert!(pods.heartbeats().is_empty(), "the pod was just created");

    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(pods.heartbeats().len(), 1, "one stamp a minute");
    tokio::time::sleep(Duration::from_secs(120)).await;
    assert_eq!(pods.heartbeats().len(), 3);
    let at = pods.heartbeats()[0];
    assert!(
        (at - now()).abs() < 5,
        "stamped with the current time: {at}"
    );

    drop(shell);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let stamped = pods.heartbeats().len();
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(
        pods.heartbeats().len(),
        stamped,
        "a closed shell stops stamping, so the sweep can take its pod if the delete failed"
    );
}

#[tokio::test(start_paused = true)]
async fn a_shell_that_cannot_stamp_keeps_working() {
    let (pods, _deleted) = FakePods::new();
    *pods.heartbeat_error.lock() = Some((ErrorKind::Forbidden, "cannot patch pods"));
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script()
        .exec
        .push(Ok(ExecScript::new().exit_when_told()));
    let shell = open(&exec, &pods).await.expect("open");
    tokio::time::sleep(Duration::from_secs(200)).await;
    assert_eq!(pods.heartbeats().len(), 3, "it keeps trying");
    assert!(pods.deletions().is_empty(), "and the shell stays open");
    drop(shell);
}

#[tokio::test]
async fn release_deletes_every_open_shells_pod_and_waits_for_the_answers() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let live = Arc::<node_shell::LiveShells>::default();
    let exec = FakeExecStreamPort::new();
    for _ in 0..2 {
        exec.script()
            .exec
            .push(Ok(ExecScript::new().exit_when_told()));
    }
    let first = node_shell::open(&exec, pods.clone(), &live, &spec())
        .await
        .expect("open");
    let second = node_shell::open(&exec, pods.clone(), &live, &spec())
        .await
        .expect("open");
    assert_eq!(live.len(), 2);
    assert!(pods.deletions().is_empty());

    assert_eq!(live.release(pods.as_ref()).await, 2);
    assert_eq!(
        pods.deletions().len(),
        2,
        "both deletes were answered before release returned"
    );

    // Dropping the shells afterwards deletes again (harmless: a gone pod is not an error) and
    // empties the list.
    drop((first, second));
    tokio::time::timeout(Duration::from_secs(5), async {
        while live.len() > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the list empties");
}

#[tokio::test]
async fn a_shell_that_ended_is_not_released_again() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let live = Arc::<node_shell::LiveShells>::default();
    let exec = FakeExecStreamPort::new();
    exec.script().exec.push(Ok(ExecScript::new()));
    let shell = node_shell::open(&exec, pods.clone(), &live, &spec())
        .await
        .expect("open");
    shell.session.status.await.expect("status");
    assert_eq!(live.len(), 0);
    assert_eq!(live.release(pods.as_ref()).await, 0);
    assert_eq!(pods.deletions(), [POD], "only the shell's own delete");
}

#[tokio::test]
async fn the_settings_template_reaches_the_pod_and_the_command() {
    let (pods, _deleted) = FakePods::new();
    let pods = Arc::new(pods);
    let exec = FakeExecStreamPort::new();
    exec.script().exec.push(Ok(ExecScript::new()));
    let spec = NodeShellSpec {
        image: "registry.local/tools:1".into(),
        image_pull_policy: Some("Always".into()),
        nsenter_args: vec!["-t".into(), "1".into(), "-m".into()],
        labels: [("team".to_owned(), "infra".to_owned())].into(),
        tolerations: vec![oxikube_ports::NodeShellToleration {
            key: Some("dedicated".into()),
            operator: Some("Exists".into()),
            ..Default::default()
        }],
        max_lifetime: Duration::from_secs(900),
        ..spec()
    };
    node_shell::open(&exec, pods.clone(), &Arc::default(), &spec)
        .await
        .expect("open");

    let Call::Create {
        namespace,
        manifest,
    } = &pods.calls()[0]
    else {
        panic!("expected the create first");
    };
    assert_eq!(namespace, "debug");
    assert_eq!(manifest["metadata"]["labels"]["team"], "infra");
    assert_eq!(manifest["metadata"]["labels"][NODE_SHELL_LABEL], "true");
    assert_eq!(manifest["spec"]["activeDeadlineSeconds"], 900);
    assert_eq!(manifest["spec"]["tolerations"][0]["key"], "dedicated");
    let container = &manifest["spec"]["containers"][0];
    assert_eq!(container["image"], "registry.local/tools:1");
    assert_eq!(container["imagePullPolicy"], "Always");

    let ExecStreamCall::Exec { command, .. } = &exec.recorded_calls()[0] else {
        panic!("expected an exec");
    };
    assert_eq!(&command[..5], ["nsenter", "-t", "1", "-m", "--"]);
}
