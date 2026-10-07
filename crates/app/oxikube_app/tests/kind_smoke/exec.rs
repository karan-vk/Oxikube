//! `ExecService` (E09-S08) over the real `ExecPort` adapter: shells in pods of a kind cluster.
//!
//! * a pod with two containers, one annotated as the default: the plan asks, the annotation is
//!   preselected, the last choice is remembered;
//! * a busybox container has `sh` and no `bash`: the probe finds that out with a quick exec and the
//!   shell that opens is `sh`, announced in the terminal's first line, and it runs commands;
//! * a shell opened without a container lands in the annotated default container;
//! * an image with no shell at all (`pause`) is an `Unsupported` error that points to a debug
//!   container, not a hung terminal;
//! * a pod that does not exist is `NotFound`.
//!
//! Every pod lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::exec::{ContainerPlan, DEFAULT_CONTAINER_ANNOTATION, ExecService, ShellOptions};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::{BackendEvent, TerminalBackend};
use oxikube_testkit::FakeClusterSourcePort;
use oxikube_testkit::images::{BUSYBOX, PAUSE};
use oxikube_testkit::integration::TestNamespace;

use crate::DEADLINE;
use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn sleeper(name: &str, containers: &[(&str, &str)], default: Option<&str>) -> Pod {
    let containers: Vec<_> = containers
        .iter()
        .map(|(container, who)| {
            serde_json::json!({
                "name": container,
                "image": BUSYBOX,
                "command": ["sleep", "3600"],
                "env": [{ "name": "WHO", "value": who }],
            })
        })
        .collect();
    let annotations = default
        .map(|d| serde_json::json!({ DEFAULT_CONTAINER_ANNOTATION: d }))
        .unwrap_or_default();
    serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "annotations": annotations,
                      "labels": { "oxikube.test/suite": "exec-service" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": containers,
        },
    }))
    .expect("a pod")
}

fn pause_pod(name: &str) -> Pod {
    serde_json::from_value(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { "oxikube.test/suite": "exec-service" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{ "name": "pause", "image": PAUSE }],
        },
    }))
    .expect("a pod")
}

async fn running(pods: &Api<Pod>, name: &str) {
    crate::eventually(
        "the pod to run",
        || String::new(),
        || async {
            pods.get(name)
                .await
                .ok()
                .and_then(|p| p.status)
                .and_then(|s| s.phase)
                .is_some_and(|phase| phase == "Running")
        },
    )
    .await;
}

/// Reads output until `marker` shows (or the deadline passes) and returns everything read.
async fn read_until(
    events: &mut futures::stream::BoxStream<'static, BackendEvent>,
    marker: &str,
) -> String {
    let mut text = String::new();
    let read = async {
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    text.push_str(&String::from_utf8_lossy(&bytes));
                    if text.contains(marker) {
                        return;
                    }
                }
                BackendEvent::Exited(status) => panic!("the shell ended early: {status:?}\n{text}"),
                BackendEvent::Error(error) => panic!("the stream failed: {error}\n{text}"),
            }
        }
    };
    tokio::time::timeout(DEADLINE, read)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {marker:?}; read:\n{text}"));
    text
}

async fn run_line(
    backend: &dyn TerminalBackend,
    events: &mut futures::stream::BoxStream<'static, BackendEvent>,
    line: &str,
    marker: &str,
) -> String {
    backend
        .write(format!("{line}\n").as_bytes())
        .await
        .expect("type a line");
    read_until(events, marker).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shell_opens_with_the_fallback_the_default_container_and_readable_failures() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    let pp = PostParams::default();
    pods.create(
        &pp,
        &sleeper("two", &[("main", "main"), ("side", "side")], Some("side")),
    )
    .await
    .expect("create the two-container pod");
    pods.create(&pp, &pause_pod("nosh"))
        .await
        .expect("create the pause pod");
    running(&pods, "two").await;
    running(&pods, "nosh").await;

    let admin = catalog_entry(&kind.context);
    let manager = ClusterSessionManager::with_config(
        Arc::new(KubeConnector::new(
            kind.kubeconfig.clone(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        )),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    manager.open(&admin, SessionOptions::default());
    manager.connect(&admin.cluster).await.expect("connect");
    let service = ExecService::new(manager);
    let pod = |name: &str| {
        ResourceRef::namespaced(
            admin.cluster.clone(),
            Gvk::new("", "v1", "Pod"),
            ns.name(),
            name,
        )
    };

    // --- the plan: two containers ask, the annotation is preselected, the last choice wins ------
    let ContainerPlan::Pick(choices) = service.plan(&pod("two"), None).await.expect("plan") else {
        panic!("two containers must ask");
    };
    let names: Vec<_> = choices
        .containers
        .iter()
        .map(|c| c.name.to_string())
        .collect();
    assert_eq!(names, ["main", "side"]);
    assert_eq!(&*choices.preselected().name, "side");
    service.remember(&pod("two"), "main");
    let ContainerPlan::Pick(again) = service.plan(&pod("two"), None).await.expect("plan") else {
        panic!("two containers must ask");
    };
    assert_eq!(
        &*again.preselected().name,
        "main",
        "the last choice for the pod"
    );
    assert_eq!(
        service.plan(&pod("nosh"), None).await.expect("plan"),
        ContainerPlan::Open("pause".into()),
        "one container opens directly"
    );

    // --- busybox: no bash, so sh; the notice says so; the shell runs commands ------------------
    let started = std::time::Instant::now();
    let backend = service
        .open_shell(&pod("two"), Some("main"), &ShellOptions::default())
        .await
        .expect("a shell in busybox");
    let mut events = backend.output_stream();
    let notice = read_until(&mut events, "using sh").await;
    // Two probes (bash, sh) and the session, to the first line on screen: the budget is 1 s on a
    // warm cluster (the story's performance note); the assertion leaves room for a busy CI node.
    let to_first_line = started.elapsed();
    eprintln!("open_shell with a bash miss: {to_first_line:?} to the first line");
    assert!(to_first_line < Duration::from_secs(5), "{to_first_line:?}");
    assert!(
        notice.contains("bash not found, using sh in two/main"),
        "the first line names the shell: {notice:?}"
    );
    let out = run_line(
        &*backend,
        &mut events,
        "echo who=$WHO sum=$((40+2))",
        "sum=42",
    )
    .await;
    assert!(out.contains("who=main"), "{out:?}");
    backend.write(b"exit\n").await.expect("exit");
    let ended = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(event) = events.next().await {
            if let BackendEvent::Exited(status) = event {
                return Some(status);
            }
        }
        None
    })
    .await
    .expect("the shell ends")
    .expect("an exit status");
    assert!(ended.is_success(), "{ended:?}");

    // --- no container named: the annotated default ---------------------------------------------
    let backend = service
        .open_shell(&pod("two"), None, &ShellOptions::default())
        .await
        .expect("a shell in the default container");
    let mut events = backend.output_stream();
    let out = run_line(
        &*backend,
        &mut events,
        "echo who=$WHO end=$((1+1))",
        "end=2",
    )
    .await;
    assert!(
        out.contains("who=side"),
        "the annotation picked the container: {out:?}"
    );
    assert!(out.contains("two/side"), "{out:?}");
    backend.kill().await.expect("kill");

    // --- a shell that does not exist anywhere in the container --------------------------------
    let Err(error) = service
        .open_shell(&pod("nosh"), None, &ShellOptions::default())
        .await
    else {
        panic!("the pause image has no shell");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
    assert!(error.message().contains("No shell (bash, sh)"), "{error}");
    assert!(error.message().contains("debug container"), "{error}");

    // --- a pod that is not there ----------------------------------------------------------------
    let Err(error) = service
        .open_shell(&pod("ghost"), None, &ShellOptions::default())
        .await
    else {
        panic!("a missing pod must not open");
    };
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}
