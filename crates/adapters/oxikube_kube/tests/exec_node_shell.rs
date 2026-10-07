//! Kind integration for E04-S09 node shells and debug containers: the shell pod is created
//! on the target node and is gone after the shell exits, after the session is dropped and
//! after the task is aborted; a pod that cannot start is cleaned up; leftovers are swept;
//! an ephemeral debug container is attached to. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::SinkExt;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::ErrorKind;
use oxikube_kube::{EphemeralContainerSpec, KubeExec, NodeShellSession};
use oxikube_ports::{NodeShellSpec, node_shell_manifest};
use oxikube_testkit::images;
use oxikube_testkit::integration::TestNamespace;

use common::exec::{BUSYBOX, create_sleeper, read_until};
use common::portforward::wait_ready;
use common::{DEADLINE, wait_until};

/// The pod, if it is still there and not already being deleted.
async fn live(client: &Client, namespace: &str, name: &str) -> Option<Pod> {
    let pod = Api::<Pod>::namespaced(client.clone(), namespace)
        .get_opt(name)
        .await
        .expect("read the pod");
    pod.filter(|p| p.metadata.deletion_timestamp.is_none())
}

async fn wait_gone(client: &Client, namespace: &str, name: &str) {
    wait_until(
        &format!("pod {name} to be deleted"),
        Duration::from_secs(60),
        || async { live(client, namespace, name).await.is_none().then_some(()) },
    )
    .await;
}

/// The cluster's first node: the single kind node.
async fn a_node(client: &Client) -> String {
    let nodes = Api::<Node>::all(client.clone())
        .list(&kube::api::ListParams::default())
        .await
        .expect("list nodes");
    nodes.items[0].metadata.name.clone().expect("node name")
}

fn spec(node: &str, namespace: &str) -> NodeShellSpec {
    NodeShellSpec {
        namespace: namespace.into(),
        // The shell image is pulled into the node by `cargo xtask kind-up`; the adapter's own
        // start timeout still covers a pull, and `pod_waits.rs` checks the default image is listed.
        image: images::BUSYBOX.into(),
        ..NodeShellSpec::new(node)
    }
}

async fn open_shell(exec: &KubeExec, node: &str, namespace: &str) -> NodeShellSession {
    exec.node_shell(&spec(node, namespace))
        .await
        .expect("open the node shell")
}

#[tokio::test]
async fn the_shell_runs_on_the_target_node_and_the_pod_goes_when_it_exits() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    let mut shell = open_shell(&exec, &node, ns.name()).await;
    let pod = live(&client, ns.name(), &shell.pod)
        .await
        .expect("the shell pod exists");
    assert_eq!(
        pod.spec.as_ref().and_then(|s| s.node_name.as_deref()),
        Some(node.as_str())
    );
    let privileged = pod
        .spec
        .as_ref()
        .and_then(|s| s.containers[0].security_context.as_ref());
    assert_eq!(privileged.and_then(|c| c.privileged), Some(true));

    let mut stdin = shell.session.stdin.take().expect("stdin");
    let mut stdout = shell.session.stdout.take().expect("stdout");
    // Inside the node's namespaces the hostname is the node's, not the pod's.
    stdin
        .send(b"echo host=$(hostname)=end\n".to_vec())
        .await
        .expect("send");
    read_until(&mut stdout, &format!("host={node}=end")).await;
    stdin.send(b"exit\n".to_vec()).await.expect("send");
    let status = shell.session.status.await.expect("status");
    assert!(status.is_success(), "{status:?}");
    assert!(
        live(&client, ns.name(), &shell.pod).await.is_none(),
        "the pod is deleted by the time the status resolves"
    );
}

#[tokio::test]
async fn dropping_the_session_removes_the_pod() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    let shell = open_shell(&exec, &node, ns.name()).await;
    let name = shell.pod.clone();
    assert!(live(&client, ns.name(), &name).await.is_some());
    drop(shell);
    wait_gone(&client, ns.name(), &name).await;
}

#[tokio::test]
async fn releasing_the_open_shells_removes_their_pods_while_the_sessions_live() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    // The app's quit: the tabs are never dropped, the adapter deletes what it still has open.
    let shell = open_shell(&exec, &node, ns.name()).await;
    let name = shell.pod.clone();
    assert!(live(&client, ns.name(), &name).await.is_some());
    assert_eq!(exec.release_node_shells().await, 1);
    wait_gone(&client, ns.name(), &name).await;
    drop(shell);
}

#[tokio::test]
async fn aborting_the_task_that_owns_the_session_removes_the_pod() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    let shell = open_shell(&exec, &node, ns.name()).await;
    let name = shell.pod.clone();
    let task = tokio::spawn(shell.session.status);
    tokio::time::sleep(Duration::from_millis(200)).await;
    task.abort();
    assert!(task.await.expect_err("aborted").is_cancelled());
    wait_gone(&client, ns.name(), &name).await;
}

#[tokio::test]
async fn a_pod_that_cannot_start_is_removed_and_reported() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    let bad_image = NodeShellSpec {
        image: "registry.invalid/oxikube/none:0".into(),
        ..spec(&node, ns.name())
    };
    let err = exec
        .node_shell(&bad_image)
        .await
        .expect_err("the image cannot be pulled");
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");

    wait_until("no shell pod is left", Duration::from_secs(60), || async {
        let pods = Api::<Pod>::namespaced((*client).clone(), ns.name())
            .list(&kube::api::ListParams::default())
            .await
            .expect("list");
        pods.items
            .iter()
            .all(|p| p.metadata.deletion_timestamp.is_some())
            .then_some(())
    })
    .await;
}

#[tokio::test]
async fn the_sweep_removes_leftover_shell_pods() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let node = a_node(&client).await;
    let exec = KubeExec::new((*client).clone());

    // What a crashed run leaves behind: a shell pod nobody owns.
    let manifest = node_shell_manifest(&spec(&node, ns.name())).expect("manifest");
    let leftover: Pod = serde_json::from_value(manifest).expect("pod");
    let pods = Api::<Pod>::namespaced((*client).clone(), ns.name());
    let created = pods
        .create(&PostParams::default(), &leftover)
        .await
        .expect("create");
    let name = created.metadata.name.expect("name");

    // Too young to be a leftover.
    let kept = exec
        .sweep_node_shells(ns.name(), Duration::from_secs(3600))
        .await
        .expect("sweep");
    assert!(kept.is_empty(), "{kept:?}");
    assert!(live(&client, ns.name(), &name).await.is_some());

    let swept = exec
        .sweep_node_shells(ns.name(), Duration::ZERO)
        .await
        .expect("sweep");
    assert_eq!(swept, std::slice::from_ref(&name));
    wait_gone(&client, ns.name(), &name).await;
}

#[tokio::test]
async fn an_ephemeral_debug_container_is_attached_to() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_sleeper(&client, ns.name(), "box").await;
    wait_ready(&client, ns.name(), "box").await;
    let exec = KubeExec::new((*client).clone());

    let spec = EphemeralContainerSpec {
        name: "dbg".into(),
        image: BUSYBOX.into(),
        target_container: Some("main".into()),
        stdin: true,
        tty: true,
        ..EphemeralContainerSpec::default()
    };
    let mut session = exec
        .debug_container(ns.name(), "box", &spec, Duration::from_secs(120))
        .await
        .expect("debug container");
    let mut stdin = session.stdin.take().expect("stdin");
    let mut stdout = session.stdout.take().expect("stdout");
    stdin
        .send(b"echo dbg-$((20+1))\n".to_vec())
        .await
        .expect("send");
    read_until(&mut stdout, "dbg-21").await;
    // The target container's process is visible: the debug container shares its namespace.
    stdin
        .send(b"echo seen=$(ps | grep -c 'sleep 360[0]')=end\n".to_vec())
        .await
        .expect("send");
    read_until(&mut stdout, "seen=1=end").await;
    stdin.send(b"exit\n".to_vec()).await.expect("send");
    assert!(session.status.await.expect("status").is_success());

    let pod = live(&client, ns.name(), "box").await.expect("pod");
    let ephemeral = pod
        .spec
        .and_then(|s| s.ephemeral_containers)
        .unwrap_or_default();
    assert_eq!(ephemeral.len(), 1);
    assert_eq!(ephemeral[0].name, "dbg");

    // The shell exited, so the container is done; it cannot be started again.
    wait_until("the debug container to be terminated", DEADLINE, || async {
        let pod = live(&client, ns.name(), "box").await?;
        let statuses = pod.status?.ephemeral_container_statuses?;
        statuses
            .iter()
            .any(|s| s.state.as_ref().is_some_and(|st| st.terminated.is_some()))
            .then_some(())
    })
    .await;
    let finished = exec
        .debug_container(ns.name(), "box", &spec, Duration::from_secs(30))
        .await
        .expect_err("the container already exited");
    assert_eq!(finished.kind(), ErrorKind::Conflict, "{finished}");
    assert!(finished.message().contains("exited"), "{finished}");
    // Ephemeral containers cannot be changed: the same name with another image is refused.
    let changed = EphemeralContainerSpec {
        image: images::E2E_BUSYBOX.into(),
        ..spec.clone()
    };
    let refused = exec
        .debug_container(ns.name(), "box", &changed, Duration::from_secs(30))
        .await
        .expect_err("an ephemeral container cannot be changed");
    assert_eq!(refused.kind(), ErrorKind::Validation, "{refused}");

    let missing = exec
        .debug_container(ns.name(), "ghost", &spec, Duration::from_secs(30))
        .await
        .expect_err("no such pod");
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");
}
