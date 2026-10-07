//! Kind integration for E09-S01: the `ExecPort` surface (exec, attach, debug container, node
//! shell) hands out `TerminalBackend`s that round-trip bytes, apply resizes, report the exit
//! and clean up on kill. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly
//! otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Api, Client};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_kube::KubeExec;
use oxikube_ports::{
    AttachTarget, BackendEvent, DebugContainerSpec, ExecPort, ExecTarget, ExitStatus,
    NodeShellSpec, TerminalSize,
};
use oxikube_testkit::integration::TestNamespace;

use common::exec::{BUSYBOX, OUTPUT_DEADLINE, create_cat, create_sleeper};
use common::portforward::wait_ready;
use common::wait_until;

fn pod_ref(namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::new(
        ClusterId::new("kubeconfig", &ContextName::new("kind")),
        Gvk::new("", "v1", "Pod"),
        Some(namespace.into()),
        name,
    )
}

/// Reads events until the output contains `marker`; returns everything read so far.
async fn read_until(events: &mut BoxStream<'static, BackendEvent>, marker: &str) -> String {
    let mut seen = Vec::new();
    let found = tokio::time::timeout(OUTPUT_DEADLINE, async {
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => seen.extend_from_slice(&bytes),
                BackendEvent::Error(error) => panic!("transport error: {error}"),
                BackendEvent::Exited(status) => panic!("exited early: {status:?}"),
            }
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

/// Drains the stream; returns the exit status it ended with.
async fn exit_of(events: &mut BoxStream<'static, BackendEvent>) -> ExitStatus {
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

#[tokio::test]
async fn exec_gives_a_tty_with_resize_and_an_exit_code() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());

    let target =
        ExecTarget::interactive(pod_ref(ns.name(), "box"), vec!["sh".into()]).container("main");
    let backend = exec.exec(&target).await.expect("exec");
    let mut events = backend.output_stream();

    backend.write(b"echo oxi-$((40+2))\n").await.expect("write");
    read_until(&mut events, "oxi-42").await;

    // The kubelet applies a resize asynchronously: ask until the shell sees it.
    let mut attempts = 0;
    loop {
        attempts += 1;
        backend
            .resize(TerminalSize::new(100, 30))
            .await
            .expect("resize");
        backend
            // The marker is shell-computed (`en$((0))d` prints `en0d`), so the TTY's echo of
            // this line cannot satisfy the read: only the real `stty` output does.
            .write(format!("echo size=$(stty size)=en$((0))d{attempts}\n").as_bytes())
            .await
            .expect("write");
        let text = read_until(&mut events, &format!("=en0d{attempts}")).await;
        if text.contains("size=30 100=en0d") {
            break;
        }
        assert!(attempts < 10, "the resize was never applied: {text:?}");
    }

    backend.write(b"exit 3\n").await.expect("write");
    let status = exit_of(&mut events).await;
    assert_eq!(status.code, Some(3), "{status:?}");
    backend.kill().await.expect("kill after exit is fine");
}

#[tokio::test]
async fn attach_round_trips_and_kill_ends_the_stream() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_cat(&client, ns.name(), "cat").await;
    wait_ready(&client, ns.name(), "cat").await;
    let exec = KubeExec::new((*client).clone());

    let target = AttachTarget {
        tty: false,
        ..AttachTarget::interactive(pod_ref(ns.name(), "cat")).container("main")
    };
    let backend = exec.attach(&target).await.expect("attach");
    let mut events = backend.output_stream();
    backend.write(b"hello-attach\n").await.expect("write");
    read_until(&mut events, "hello-attach").await;

    backend.kill().await.expect("kill");
    backend.kill().await.expect("kill is idempotent");
    let status = exit_of(&mut events).await;
    assert_eq!(status.signal.as_deref(), Some("KILL"), "{status:?}");
    assert!(backend.write(b"x").await.is_err(), "closed after kill");
}

#[tokio::test]
async fn a_missing_pod_is_not_found() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let exec = KubeExec::new((*client).clone());
    let err = exec
        .exec(&ExecTarget::interactive(
            pod_ref(ns.name(), "ghost"),
            vec!["sh".into()],
        ))
        .await
        .err()
        .expect("no such pod");
    assert_eq!(err.kind(), oxikube_domain::ErrorKind::NotFound, "{err}");
}

#[tokio::test]
async fn a_debug_container_is_created_and_attached_through_the_port() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());

    let spec = DebugContainerSpec {
        target_container: Some("main".into()),
        start_timeout: Duration::from_secs(120),
        ..DebugContainerSpec::new(pod_ref(ns.name(), "box"), BUSYBOX)
    };
    let backend = exec.create_debug_container(&spec).await.expect("debug");
    let mut events = backend.output_stream();
    backend.write(b"echo dbg-$((20+1))\n").await.expect("write");
    read_until(&mut events, "dbg-21").await;
    backend.write(b"exit\n").await.expect("write");
    assert!(exit_of(&mut events).await.is_success());

    let pod = Api::<Pod>::namespaced((*client).clone(), ns.name())
        .get("box")
        .await
        .expect("pod");
    let ephemeral = pod
        .spec
        .and_then(|s| s.ephemeral_containers)
        .unwrap_or_default();
    assert_eq!(ephemeral.len(), 1);
    assert!(ephemeral[0].name.starts_with("debugger-"), "{ephemeral:?}");
}

async fn a_node(client: &Client) -> String {
    let nodes = Api::<Node>::all(client.clone())
        .list(&kube::api::ListParams::default())
        .await
        .expect("list nodes");
    nodes.items[0].metadata.name.clone().expect("node name")
}

#[tokio::test]
async fn a_node_shell_through_the_port_removes_its_pod_on_kill() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    let spec = NodeShellSpec {
        namespace: ns.name().into(),
        image: BUSYBOX.into(),
        start_timeout: Duration::from_secs(120),
        ..NodeShellSpec::new(&node)
    };
    let backend = ExecPort::node_shell(&exec, &spec)
        .await
        .expect("node shell");
    let mut events = backend.output_stream();
    let pods = Api::<Pod>::namespaced((*client).clone(), ns.name());
    let helper = pods.list(&Default::default()).await.expect("list").items;
    assert_eq!(helper.len(), 1, "one helper pod in the test namespace");
    let name = helper[0].metadata.name.clone().expect("pod name");

    backend
        .write(b"echo host=$(hostname)=end\n")
        .await
        .expect("write");
    read_until(&mut events, &format!("host={node}=end")).await;

    backend.kill().await.expect("kill");
    assert_eq!(exit_of(&mut events).await.signal.as_deref(), Some("KILL"));
    wait_until(
        &format!("pod {name} to be deleted"),
        Duration::from_secs(60),
        || async {
            let pod = pods.get_opt(&name).await.expect("read the pod");
            pod.is_none_or(|p| p.metadata.deletion_timestamp.is_some())
                .then_some(())
        },
    )
    .await;
}
