//! Kind scenarios of the terminal suite (E09-S13) that the per-port files do not cover: attaching
//! to a long-running pod that keeps writing, and a debug container in a pod that has no shell
//! (the stand-in for a distroless image). The echo, resize, exit-code, node-shell and
//! read-only-guard scenarios live next to their ports (`exec_terminal`, `exec_kube_stream`,
//! `exec_node_shell`, and `oxikube_app/tests/kind_smoke`); `tests/README.md` maps all of them.
//!
//! Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use oxikube_kube::KubeExec;
use oxikube_ports::{AttachTarget, DebugContainerSpec, ExecPort, ExecTarget};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::pods::{LOGGER_PREFIX, MAIN};

use common::exec::{
    BUSYBOX, create_logger, create_shell_less, events_exit, events_satisfying, events_until,
    pod_ref,
};
use common::portforward::wait_ready;

/// The numbers of the complete `tick <n>` lines in `text` (a line cut short by the end of the
/// read is not one).
fn ticks(text: &str) -> Vec<u32> {
    let complete = text.rfind('\n').map_or("", |end| &text[..end]);
    complete
        .lines()
        .filter_map(|line| line.trim().strip_prefix(LOGGER_PREFIX)?.parse().ok())
        .collect()
}

#[tokio::test]
async fn attaching_to_a_long_running_pod_receives_what_it_keeps_writing() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_logger(&client, ns.name(), "logger").await;
    wait_ready(&client, ns.name(), "logger").await;
    let exec = KubeExec::new((*client).clone());

    // The logger has no stdin and no TTY: a read-only view of its output, like `kubectl attach`.
    let target = AttachTarget {
        tty: false,
        stdin: false,
        ..AttachTarget::interactive(pod_ref(ns.name(), "logger")).container(MAIN)
    };
    let backend = exec.attach(&target).await.expect("attach");
    let mut events = backend.output_stream();
    // Attach shows what is written from now on, not the history: four complete ticks in a row,
    // in order, none skipped or repeated.
    let text = events_satisfying(&mut events, "four ticks", |text| ticks(text).len() >= 4).await;
    let seen = ticks(&text);
    assert!(
        seen.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "ticks arrive in order, none skipped or repeated: {seen:?}"
    );
    // Closing the attach leaves the pod running: it only ends this client's view.
    backend.kill().await.expect("kill");
    assert!(events_exit(&mut events).await.signal.is_some());
    let pod = Api::<Pod>::namespaced((*client).clone(), ns.name())
        .get("logger")
        .await
        .expect("the pod");
    assert_eq!(
        pod.status.and_then(|s| s.phase).as_deref(),
        Some("Running"),
        "the logger keeps running after its attach closed"
    );
}

#[tokio::test]
async fn a_debug_container_reaches_a_pod_that_has_no_shell() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_shell_less(&client, ns.name(), "bare").await;
    wait_ready(&client, ns.name(), "bare").await;
    let exec = KubeExec::new((*client).clone());

    // A shell in the pod itself finds nothing to run: the image has no `sh`, so the session opens
    // (the API server accepts the exec) and ends at once with a failure status.
    let shell =
        ExecTarget::interactive(pod_ref(ns.name(), "bare"), vec!["sh".into()]).container(MAIN);
    let backend = exec
        .exec(&shell)
        .await
        .expect("the exec itself is accepted");
    let mut events = backend.output_stream();
    let status = events_exit(&mut events).await;
    assert!(
        !status.is_success(),
        "no shell in the pause image: {status:?}"
    );

    // The debug container brings its own tools and shares the target's processes.
    let spec = DebugContainerSpec {
        target_container: Some(MAIN.into()),
        start_timeout: Duration::from_secs(120),
        ..DebugContainerSpec::new(pod_ref(ns.name(), "bare"), BUSYBOX)
    };
    let backend = exec.create_debug_container(&spec).await.expect("debug");
    let mut events = backend.output_stream();
    backend
        .write(b"echo seen=$(ps | grep -c '[/]pause')=end\n")
        .await
        .expect("write");
    events_until(&mut events, "seen=1=end").await;
    backend.write(b"exit\n").await.expect("write");
    assert!(events_exit(&mut events).await.is_success());

    let pod = Api::<Pod>::namespaced((*client).clone(), ns.name())
        .get("bare")
        .await
        .expect("pod");
    let ephemeral = pod
        .spec
        .and_then(|s| s.ephemeral_containers)
        .unwrap_or_default();
    assert_eq!(ephemeral.len(), 1);
    assert_eq!(ephemeral[0].target_container_name.as_deref(), Some(MAIN));
}
