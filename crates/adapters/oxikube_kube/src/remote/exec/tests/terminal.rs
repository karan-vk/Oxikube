//! `ExecPort` on `KubeExec`: descriptors become the right requests and refusals keep their
//! kinds. The live websocket path is `tests/exec_terminal.rs` on kind.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{
    AttachTarget, BackendEvent, DebugContainerSpec, ExecPort, ExecTarget, ExitStatus,
    NodeShellSpec, SessionBackend, TerminalBackend,
};
use oxikube_testkit::fakes::{ExecScript, FakeExecStreamPort};

use super::fakes::FakePods;
use crate::fake_api::{FakeApi, status_body};
use crate::remote::exec::node_shell;
use crate::remote::exec::terminal::debug_spec;
use crate::remote::exec::{DEFAULT_DEBUG_START_TIMEOUT, KubeExec};

const EXEC: &str = "/api/v1/namespaces/default/pods/p/exec";
const ATTACH: &str = "/api/v1/namespaces/default/pods/p/attach";

fn pod_ref(namespace: Option<&str>) -> ResourceRef {
    ResourceRef::new(
        ClusterId::new("kubeconfig", &ContextName::new("kind")),
        Gvk::new("", "v1", "Pod"),
        namespace.map(Into::into),
        "p",
    )
}

fn sh() -> Vec<String> {
    vec!["sh".into()]
}

#[tokio::test]
async fn exec_maps_the_target_onto_the_request() {
    let api = FakeApi::new();
    api.reply(EXEC, 403, status_body(403, "Forbidden", "no"));
    let exec = KubeExec::new(api.client());

    let tty = ExecTarget::interactive(pod_ref(Some("default")), sh()).container("app");
    let err = exec.exec(&tty).await.err().expect("refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    let query = api.requests()[0].query.clone();
    for expected in [
        "tty=true",
        "stdin=true",
        "stdout=true",
        "container=app",
        "command=sh",
    ] {
        assert!(query.contains(expected), "{expected} in {query}");
    }
    assert!(!query.contains("stderr"), "a TTY merges stderr: {query}");

    let plain = ExecTarget {
        tty: false,
        stdin: false,
        ..ExecTarget::interactive(pod_ref(Some("default")), sh())
    };
    exec.exec(&plain).await.err().expect("refused");
    let query = api.requests()[1].query.clone();
    assert!(query.contains("stderr=true"), "{query}");
    assert!(
        !query.contains("tty=true") && !query.contains("stdin=true"),
        "{query}"
    );
}

#[tokio::test]
async fn attach_uses_the_attach_route() {
    let api = FakeApi::new();
    api.reply(ATTACH, 403, status_body(403, "Forbidden", "no"));
    let err = KubeExec::new(api.client())
        .attach(&AttachTarget::interactive(pod_ref(Some("default"))))
        .await
        .err()
        .expect("refused");
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert_eq!(api.requests()[0].path, ATTACH);
    assert!(!api.requests()[0].query.contains("command"));
}

#[tokio::test]
async fn bad_descriptors_are_refused_before_any_request() {
    let api = FakeApi::new();
    let exec = KubeExec::new(api.client());

    let no_namespace = ExecTarget::interactive(pod_ref(None), sh());
    let empty = ExecTarget::interactive(pod_ref(Some("default")), vec![]);
    for target in [&no_namespace, &empty] {
        let err = exec.exec(target).await.err().expect("invalid");
        assert_eq!(err.kind(), ErrorKind::Validation, "{err}");
    }
    let err = exec
        .attach(&AttachTarget::interactive(pod_ref(None)))
        .await
        .err()
        .expect("invalid");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let err = exec
        .create_debug_container(&DebugContainerSpec::new(pod_ref(None), "busybox"))
        .await
        .err()
        .expect("invalid");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let err = exec
        .create_debug_container(&DebugContainerSpec::new(pod_ref(Some("default")), " "))
        .await
        .err()
        .expect("blank image");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let blank_image = NodeShellSpec {
        image: String::new(),
        ..NodeShellSpec::new("node-1")
    };
    let err = ExecPort::node_shell(&exec, &blank_image)
        .await
        .err()
        .expect("blank image");
    assert_eq!(err.kind(), ErrorKind::Validation);
    let err = ExecPort::node_shell(&exec, &NodeShellSpec::new("no/such"))
        .await
        .err()
        .expect("bad node");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(api.requests().is_empty(), "nothing reached the server");
}

#[test]
fn a_debug_container_is_interactive_and_named_when_unnamed() {
    let spec = DebugContainerSpec {
        target_container: Some("app".into()),
        command: vec!["sh".into()],
        ..DebugContainerSpec::new(pod_ref(Some("default")), "busybox:1.37")
    };
    let first = debug_spec(&spec);
    assert!(first.stdin && first.tty);
    assert_eq!(first.image, "busybox:1.37");
    assert_eq!(first.target_container.as_deref(), Some("app"));
    assert_eq!(first.command, ["sh"]);
    assert!(first.name.starts_with("debugger-") && first.name.len() == "debugger-".len() + 8);
    assert_ne!(first.name, debug_spec(&spec).name, "unique per request");
    assert_eq!(spec.start_timeout, DEFAULT_DEBUG_START_TIMEOUT);

    let named = DebugContainerSpec {
        name: Some("dbg".into()),
        ..spec
    };
    assert_eq!(debug_spec(&named).name, "dbg");
}

/// A node shell as the terminal sees it: the helper pod goes when the user kills the tab, and
/// when the shell exits by itself, and the stream reports the exit.
async fn node_backend(
    script: ExecScript,
) -> (
    Box<dyn TerminalBackend>,
    tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    let (pods, deleted) = FakePods::new();
    let exec = FakeExecStreamPort::new();
    exec.script().exec.push(Ok(script));
    let live = Arc::default();
    let shell = node_shell::open(
        &exec,
        Arc::new(pods),
        &live,
        &NodeShellSpec::new("worker-1"),
    )
    .await
    .expect("open");
    (Box::new(SessionBackend::new(shell.session)), deleted)
}

#[tokio::test]
async fn killing_a_node_shell_backend_deletes_the_helper_pod() {
    let (backend, mut deleted) = node_backend(ExecScript::new().exit_when_told()).await;
    let mut events = backend.output_stream();
    backend.kill().await.expect("kill");
    backend.kill().await.expect("kill is idempotent");
    assert!(matches!(
        events.next().await,
        Some(BackendEvent::Exited(status)) if status.signal.as_deref() == Some("KILL")
    ));
    let name = tokio::time::timeout(Duration::from_secs(5), deleted.recv())
        .await
        .expect("the cleanup runs");
    assert!(name.is_some_and(|n| n.starts_with("oxikube-node-shell-")));
}

#[tokio::test]
async fn a_node_shell_that_exits_reports_it_and_cleans_up() {
    let (backend, mut deleted) = node_backend(
        ExecScript::new()
            .stdout("bye\n")
            .exit(Ok(ExitStatus::with_code(0))),
    )
    .await;
    let mut events = backend.output_stream();
    let mut output = Vec::new();
    let mut exit = None;
    while let Some(event) = events.next().await {
        match event {
            BackendEvent::Output(bytes) => output.extend_from_slice(&bytes),
            BackendEvent::Exited(status) => exit = Some(status),
            BackendEvent::Error(error) => panic!("{error}"),
        }
    }
    assert_eq!(output, b"bye\n");
    assert!(exit.expect("exit event").is_success());
    assert!(deleted.recv().await.is_some(), "the pod was deleted");
}
