//! `FakeTerminalBackend` and `FakeExecPort`.

use std::time::Duration;

use futures::FutureExt;
use futures::StreamExt;
use futures::executor::block_on;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{
    AttachTarget, BackendEvent, DebugContainerSpec, ExecPort, ExecTarget, ExitStatus,
    NodeShellSpec, TerminalBackend, TerminalSize,
};

use super::*;

fn pod() -> ResourceRef {
    ResourceRef::new(
        ClusterId::new("kubeconfig", &ContextName::new("kind")),
        Gvk::new("", "v1", "Pod"),
        Some("demo".into()),
        "web-0",
    )
}

fn output(event: Option<BackendEvent>) -> Vec<u8> {
    match event {
        Some(BackendEvent::Output(bytes)) => bytes.to_vec(),
        other => panic!("expected output, got {other:?}"),
    }
}

#[test]
fn echo_round_trip() {
    block_on(async {
        let fake = FakeTerminalBackend::echo();
        let backend: Box<dyn TerminalBackend> = Box::new(fake.clone());
        let mut events = backend.output_stream();
        backend.write(b"ls\r").await.unwrap();
        backend.write(b"pwd\r").await.unwrap();
        assert_eq!(output(events.next().await), b"ls\r");
        assert_eq!(output(events.next().await), b"pwd\r");
        assert_eq!(fake.written(), b"ls\rpwd\r");
        assert_eq!(fake.writes().len(), 2);
    });
}

#[test]
fn silent_does_not_echo() {
    block_on(async {
        let fake = FakeTerminalBackend::silent();
        let mut events = fake.output_stream();
        fake.write(b"secret").await.unwrap();
        assert!((&mut events.next()).now_or_never().is_none());
        fake.output("prompt$ ");
        assert_eq!(output(events.next().await), b"prompt$ ");
    });
}

#[test]
fn scripted_output_follows_the_clock_in_order() {
    block_on(async {
        let fake = FakeTerminalBackend::silent()
            .output_at(Duration::from_millis(200), "second")
            .output_at(Duration::from_millis(100), "first")
            .exit_at(Duration::from_millis(300), ExitStatus::with_code(1));
        let mut events = fake.output_stream();
        assert!(
            events.next().now_or_never().is_none(),
            "nothing before time moves"
        );
        fake.clock().advance(Duration::from_millis(100));
        assert_eq!(output(events.next().await), b"first");
        assert!(events.next().now_or_never().is_none());
        fake.clock().advance(Duration::from_millis(100));
        assert_eq!(output(events.next().await), b"second");
        fake.clock().advance(Duration::from_millis(100));
        match events.next().await {
            Some(BackendEvent::Exited(status)) => assert_eq!(status.code, Some(1)),
            other => panic!("expected exit, got {other:?}"),
        }
        assert!(events.next().await.is_none(), "the exit ends the stream");
        assert!(fake.is_closed());
    });
}

#[test]
fn scripted_errors_are_events() {
    block_on(async {
        let fake = FakeTerminalBackend::silent()
            .error_at(Duration::ZERO, OxiError::network("reset"))
            .exit_at(Duration::ZERO, ExitStatus::default());
        let mut events = fake.output_stream();
        assert!(
            matches!(events.next().await, Some(BackendEvent::Error(e)) if e.kind() == ErrorKind::Network)
        );
        assert!(matches!(events.next().await, Some(BackendEvent::Exited(_))));
        assert!(events.next().await.is_none());
    });
}

#[test]
fn live_exit_ends_the_stream_and_closes_writes() {
    block_on(async {
        let fake = FakeTerminalBackend::echo();
        let mut events = fake.output_stream();
        assert!(fake.output("bye"));
        assert!(fake.exit(ExitStatus::success()));
        assert!(
            !fake.exit(ExitStatus::success()),
            "a second exit is ignored"
        );
        assert!(!fake.output("late"));
        assert_eq!(output(events.next().await), b"bye");
        assert!(matches!(events.next().await, Some(BackendEvent::Exited(s)) if s.is_success()));
        assert!(events.next().await.is_none());
        let err = fake.write(b"x").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
        let err = fake.resize(TerminalSize::new(1, 1)).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
    });
}

#[test]
fn resize_is_recorded() {
    block_on(async {
        let fake = FakeTerminalBackend::echo();
        fake.resize(TerminalSize::new(80, 24)).await.unwrap();
        fake.resize(TerminalSize::new(120, 40).with_pixels(960, 800))
            .await
            .unwrap();
        assert_eq!(
            fake.resizes(),
            vec![
                TerminalSize::new(80, 24),
                TerminalSize::new(120, 40).with_pixels(960, 800)
            ]
        );
    });
}

#[test]
fn kill_is_idempotent_and_ends_with_a_kill_signal() {
    block_on(async {
        let fake = FakeTerminalBackend::echo();
        let mut events = fake.output_stream();
        fake.kill().await.unwrap();
        fake.kill().await.unwrap();
        assert_eq!(fake.kill_count(), 2);
        match events.next().await {
            Some(BackendEvent::Exited(status)) => {
                assert_eq!(status.signal.as_deref(), Some("KILL"));
            }
            other => panic!("expected exit, got {other:?}"),
        }
        assert!(events.next().await.is_none());
        assert_eq!(
            fake.write(b"x").await.unwrap_err().kind(),
            ErrorKind::Conflict
        );
    });
}

#[test]
fn output_stream_is_single_consumer() {
    block_on(async {
        let fake = FakeTerminalBackend::echo();
        let _first = fake.output_stream();
        assert!(fake.output_stream().next().await.is_none());
    });
}

#[test]
fn a_failed_write_is_reported_once() {
    block_on(async {
        let fake = FakeTerminalBackend::echo();
        fake.fail_next_write(OxiError::network("pipe closed"));
        assert_eq!(
            fake.write(b"a").await.unwrap_err().kind(),
            ErrorKind::Network
        );
        fake.write(b"b").await.unwrap();
        assert_eq!(fake.writes(), vec![b"a".to_vec(), b"b".to_vec()]);
    });
}

#[test]
fn exec_port_records_descriptors_and_returns_the_scripted_backend() {
    block_on(async {
        let port = FakeExecPort::new();
        let scripted = FakeTerminalBackend::silent();
        port.script().exec.push_ok(scripted.clone());
        let target = ExecTarget::interactive(pod(), vec!["sh".into()]).container("app");
        let backend = port.exec(&target).await.unwrap();
        backend.write(b"id\n").await.unwrap();
        assert_eq!(scripted.written(), b"id\n");

        let attach = AttachTarget::interactive(pod());
        let debug = DebugContainerSpec::new(pod(), "busybox:1.37");
        let node = NodeShellSpec::new("node-1");
        port.attach(&attach).await.unwrap();
        port.create_debug_container(&debug).await.unwrap();
        port.node_shell(&node).await.unwrap();
        assert_eq!(
            port.recorded_calls(),
            vec![
                ExecPortCall::Exec(target),
                ExecPortCall::Attach(attach),
                ExecPortCall::CreateDebugContainer(debug),
                ExecPortCall::NodeShell(node),
            ]
        );
        assert_eq!(port.opened().len(), 4);
    });
}

#[test]
fn exec_port_errors_are_scripted() {
    block_on(async {
        let port = FakeExecPort::new();
        port.script()
            .node_shell
            .push_err(OxiError::forbidden("pods is forbidden"));
        let err = port
            .node_shell(&NodeShellSpec::new("n"))
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind(), ErrorKind::Forbidden);
        // The queue is spent: the fallback is an echoing backend.
        assert!(port.node_shell(&NodeShellSpec::new("n")).await.is_ok());
    });
}

#[test]
fn descriptors_without_a_namespace_are_rejected_by_the_helper() {
    let mut cluster_scoped = pod();
    cluster_scoped.namespace = None;
    let target = ExecTarget::interactive(cluster_scoped, vec!["sh".into()]);
    assert_eq!(
        target.namespaced_pod().unwrap_err().kind(),
        ErrorKind::Validation
    );
    assert_eq!(
        ExecTarget::interactive(pod(), vec![])
            .namespaced_pod()
            .unwrap(),
        ("demo", "web-0")
    );
}
