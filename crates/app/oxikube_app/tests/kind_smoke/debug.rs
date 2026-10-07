//! Ephemeral debug containers (E09-S10) over the real `ExecPort` adapter: `pod::Debug` through the
//! `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a busybox pod gets a debug container (default image, the target container named): the dialog's
//!   `DebugRunner` confirms, the guard audits with the image and target, the container is attached
//!   once it runs, `ps` in it shows the target's process (the shared process namespace), and after
//!   the shell exits the pod lists the ephemeral container and its status;
//! * a pod whose image has no shell at all (`pause`) refuses `open_shell` but takes a debug
//!   container, which is the reason the feature exists;
//! * a container that cannot start (an image that does not exist) is a readable error, not a hang;
//! * a read-only session refuses the command and adds nothing to the pod.
//!
//! Every pod lives in the test's own `oxi-test-<rand>` namespace and goes with it.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::command_bus::{CommandBus, CommandOutput, CommandRegistry, HandlerContext};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{DebugRequest, DebugRunner, ExecService, MutationGuard, ShellOptions};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::BackendEvent;
use oxikube_testkit::images::{BUSYBOX, PAUSE};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::json;

use crate::DEADLINE;
use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn pod_json(name: &str, image: &str, command: Option<[&str; 2]>) -> Pod {
    let mut container = json!({ "name": "main", "image": image });
    if let Some(command) = command {
        container["command"] = json!(command);
    }
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { "oxikube.test/suite": "debug-container" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [container],
        },
    }))
    .expect("a pod")
}

async fn running(pods: &Api<Pod>, name: &str) {
    crate::eventually("the pod to run", String::new, || async {
        pods.get(name)
            .await
            .ok()
            .and_then(|p| p.status)
            .and_then(|s| s.phase)
            .is_some_and(|phase| phase == "Running")
    })
    .await;
}

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

struct World {
    service: Arc<ExecService>,
    bus: CommandBus,
    state: Arc<FakeStatePort>,
}

/// The app services over `kind`'s session: the real `ExecService`, a bus whose `pod::Debug` handler
/// is the terminal crate's minus the tab (it adds the container under the guard's permit and
/// returns its name), and the guard on an in-memory audit log.
async fn world(kind: &Kind, read_only: bool) -> (World, ResourceRef) {
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
    manager.open(
        &admin,
        SessionOptions {
            read_only,
            ..SessionOptions::default()
        },
    );
    manager.connect(&admin.cluster).await.expect("connect");
    let service = Arc::new(ExecService::new(manager.clone()));
    let handler = service.clone();
    let mut registry = CommandRegistry::new();
    registry
        .register(
            *command::lookup(CommandId::POD_DEBUG).expect("declared"),
            move |command: Command, cx: HandlerContext| {
                let service = handler.clone();
                async move {
                    let request = DebugRequest::from_command(&command)?;
                    let opened = service.open_debug(cx.require_mutation()?, &request).await?;
                    Ok(CommandOutput {
                        message: None,
                        data: Some(json!({ "container": opened.plan.name })),
                    })
                }
            },
        )
        .expect("register pod::Debug");
    let state = Arc::new(FakeStatePort::new());
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager, state.clone(), Arc::new(TokioClock)),
    );
    let pod = ResourceRef::namespaced(
        admin.cluster,
        Gvk::new("", "v1", "Pod"),
        "placeholder",
        "placeholder",
    );
    (
        World {
            service,
            bus,
            state,
        },
        pod,
    )
}

fn in_pod(template: &ResourceRef, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        template.cluster.clone(),
        template.gvk.clone(),
        namespace,
        name,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_debug_container_shares_the_targets_processes_and_is_listed_on_the_pod() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(
        &PostParams::default(),
        &pod_json("web", BUSYBOX, Some(["sleep", "3600"])),
    )
    .await
    .expect("create the pod");
    pods.create(&PostParams::default(), &pod_json("nosh", PAUSE, None))
        .await
        .expect("create the pause pod");
    running(&pods, "web").await;
    running(&pods, "nosh").await;

    let (w, template) = world(&kind, false).await;
    let web = in_pod(&template, ns.name(), "web");
    let runner = DebugRunner::new(w.bus.clone(), "tester");

    // --- the dialog's request: defaults (busybox, the pod's default container, sh) --------------
    let defaults = w.service.debug_defaults(&web).await.expect("defaults");
    assert_eq!(
        (defaults.image.as_str(), defaults.command.as_str()),
        ("busybox", "sh")
    );
    assert_eq!(&*defaults.targets[defaults.target].name, "main");
    // The dialog starts with `busybox`; the test pulls the pinned image the suites share.
    let mut request = DebugRequest::new(web.clone(), BUSYBOX);
    request.target_container = Some(defaults.targets[defaults.target].name.to_string());
    request.start_timeout = Duration::from_secs(120);
    let report = runner.run(&request).await.expect("a debug container");
    assert!(report.container.starts_with("debugger-"), "{report:?}");

    // --- the guard: one audited mutation, with the image and the target ---------------------------
    let audit = w.state.audit_log();
    let [record] = audit.as_slice() else {
        panic!("one audit record, got {audit:?}");
    };
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(&*record.cmd, "pod::Debug");
    let detail = record.detail.as_deref().unwrap();
    assert_eq!(detail, format!("session=debug image={BUSYBOX} target=main"));

    // --- attached once running: the terminal claims the session the command opened ---------------
    let backend = w
        .service
        .attach(&web, Some(&report.container))
        .await
        .expect("the debug terminal");
    let mut events = backend.output_stream();
    let notice = read_until(&mut events, "sharing the processes of main").await;
    assert!(notice.contains(&report.container), "{notice:?}");
    backend
        .write(b"ps; echo ps-done-$((1+1))\n")
        .await
        .expect("type");
    let ps = read_until(&mut events, "ps-done-2").await;
    assert!(
        ps.contains("sleep 3600"),
        "the target's process is visible from the debug container (shared process namespace):\n{ps}"
    );
    backend.write(b"exit\n").await.expect("exit");
    let ended = tokio::time::timeout(Duration::from_secs(30), async {
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

    // --- the pod lists the ephemeral container and its status --------------------------------------
    let pod = pods.get("web").await.expect("the pod");
    let ephemeral = pod
        .spec
        .as_ref()
        .and_then(|s| s.ephemeral_containers.clone())
        .unwrap_or_default();
    assert_eq!(ephemeral.len(), 1, "{ephemeral:?}");
    assert_eq!(ephemeral[0].name, report.container);
    assert_eq!(ephemeral[0].image.as_deref(), Some(BUSYBOX));
    assert_eq!(ephemeral[0].target_container_name.as_deref(), Some("main"));
    assert!(ephemeral[0].stdin == Some(true) && ephemeral[0].tty == Some(true));
    crate::eventually("the ephemeral container's status", String::new, || async {
        pods.get("web")
            .await
            .ok()
            .and_then(|p| p.status)
            .and_then(|s| s.ephemeral_container_statuses)
            .is_some_and(|statuses| {
                statuses.iter().any(|s| {
                    s.name == report.container
                        && s.state
                            .as_ref()
                            .is_some_and(|state| state.terminated.is_some())
                })
            })
    })
    .await;

    // --- a container's name is never reused, even after it exited ----------------------------------
    let mut again = request.clone();
    again.name = Some(report.container.clone());
    let err = runner.run(&again).await.expect_err("the name is taken");
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");

    // --- no shell in the pause image: exec fails, a debug container works --------------------------
    let nosh = in_pod(&template, ns.name(), "nosh");
    let Err(error) = w
        .service
        .open_shell(&nosh, None, &ShellOptions::default())
        .await
    else {
        panic!("the pause image has no shell");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
    let mut debug = DebugRequest::new(nosh.clone(), BUSYBOX);
    debug.start_timeout = Duration::from_secs(120);
    let report = runner
        .run(&debug)
        .await
        .expect("a debug container in a shell-less pod");
    let backend = w
        .service
        .attach(&nosh, Some(&report.container))
        .await
        .expect("attach");
    let mut events = backend.output_stream();
    backend
        .write(b"ps; echo ps-done-$((2+2))\n")
        .await
        .expect("type");
    let ps = read_until(&mut events, "ps-done-4").await;
    assert!(ps.contains("/pause"), "the pause process is visible:\n{ps}");
    backend.kill().await.expect("kill");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_container_that_cannot_start_and_a_read_only_session_are_readable_errors() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    pods.create(
        &PostParams::default(),
        &pod_json("web", BUSYBOX, Some(["sleep", "3600"])),
    )
    .await
    .expect("create the pod");
    running(&pods, "web").await;

    // An image that does not exist never starts: a timeout or the pull failure, with the image's
    // name in it, within the request's own deadline.
    let (w, template) = world(&kind, false).await;
    let web = in_pod(&template, ns.name(), "web");
    let runner = DebugRunner::new(w.bus.clone(), "tester");
    let mut broken = DebugRequest::new(web.clone(), "registry.invalid/oxikube/none:0");
    broken.start_timeout = Duration::from_secs(15);
    let started = std::time::Instant::now();
    let err = runner
        .run(&broken)
        .await
        .expect_err("the image cannot be pulled");
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "no hang: {:?}",
        started.elapsed()
    );
    assert!(
        matches!(
            err.kind(),
            ErrorKind::Timeout | ErrorKind::Conflict | ErrorKind::NotFound
        ),
        "{err}"
    );
    eprintln!("BROKEN IMAGE: {:?} {err}", err.kind());
    assert!(!err.message().is_empty());
    assert_eq!(
        w.state.audit_log().last().map(|r| r.outcome),
        Some(AuditOutcome::Failed)
    );
    assert_eq!(w.service.unclaimed_debug_sessions(), 0);

    // A target the pod does not have is refused before anything is patched.
    let mut wrong = DebugRequest::new(web.clone(), BUSYBOX);
    wrong.target_container = Some("nope".into());
    let err = runner.run(&wrong).await.expect_err("no such container");
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");

    // Read-only: refused by the guard for everyone; the pod is untouched.
    let (ro, template) = world(&kind, true).await;
    let web = in_pod(&template, ns.name(), "web");
    let before = pods
        .get("web")
        .await
        .unwrap()
        .spec
        .unwrap()
        .ephemeral_containers;
    let err: oxikube_domain::OxiError = DebugRunner::new(ro.bus.clone(), "tester")
        .run(&DebugRequest::new(web, BUSYBOX))
        .await
        .expect_err("read-only");
    assert_eq!(err.kind(), ErrorKind::Forbidden, "{err}");
    let after = pods
        .get("web")
        .await
        .unwrap()
        .spec
        .unwrap()
        .ephemeral_containers;
    assert_eq!(
        before.as_ref().map(Vec::len),
        after.as_ref().map(Vec::len),
        "read-only added nothing"
    );
    assert_eq!(
        ro.state.audit_log().last().map(|r| r.outcome),
        Some(AuditOutcome::Denied)
    );
}
