//! Scale (E12-S04) against the real adapter: `workload::Scale` through the `CommandBus` and
//! `MutationGuard` on a kind cluster, no UI.
//!
//! * a Deployment in the test's own namespace scales up through the `scale` subresource and the
//!   cluster reports the new ready count; the audit record carries the count before and after;
//! * a dispatch that is itself a dry run changes nothing;
//! * scaling to zero is accepted and the pods go;
//! * a read-only session refuses and the Deployment is untouched;
//! * `WorkloadService::plan_scale` reads the current and desired counts.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace; nothing touches the node.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::Deployment;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::workloads::{WorkloadService, register_commands};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn deployment(name: &str) -> Deployment {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": name, "labels": {"app": name}},
        "spec": {
            "replicas": 1,
            "selector": {"matchLabels": {"app": name}},
            "template": {
                "metadata": {"labels": {"app": name}},
                "spec": {
                    "terminationGracePeriodSeconds": 0,
                    "containers": [{"name": "sleeper", "image": BUSYBOX,
                                    "command": ["sleep", "3600"]}]
                }
            }
        }
    }))
    .expect("a deployment")
}

async fn spec_replicas(api: &Api<Deployment>, name: &str) -> i32 {
    api.get(name)
        .await
        .expect("get")
        .spec
        .and_then(|spec| spec.replicas)
        .unwrap_or(1)
}

async fn ready(api: &Api<Deployment>, name: &str) -> i32 {
    api.get(name)
        .await
        .expect("get")
        .status
        .and_then(|status| status.ready_replicas)
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scale_up_dry_run_zero_and_read_only() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let api = Api::<Deployment>::namespaced(admin_client.clone(), ns.name());
    api.create(&PostParams::default(), &deployment("web"))
        .await
        .expect("create the deployment");

    let entry = catalog_entry(&kind.context);
    let connector = KubeConnector::new(
        kind.kubeconfig.clone(),
        PoolConfig::default(),
        ConnectorConfig::default(),
    );
    let manager = ClusterSessionManager::with_config(
        Arc::new(connector),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    let state = Arc::new(FakeStatePort::new());
    let service = WorkloadService::new(manager.clone());
    let mut registry = CommandRegistry::new();
    let handler_sessions = manager.clone();
    registry
        .install("oxikube_app::workloads", move |r| {
            register_commands(r, handler_sessions)
        })
        .expect("register workload::Scale");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    manager.open(&entry, SessionOptions::default());
    manager.connect(&entry.cluster).await.expect("connect");

    let web = ResourceRef::namespaced(
        entry.cluster.clone(),
        Gvk::new("apps", "v1", "Deployment"),
        ns.name(),
        "web",
    );
    let scale = |replicas| Command::WorkloadScale {
        target: web.clone(),
        replicas,
    };
    let ctx = || DispatchContext::new(Initiator::Ui, "tester").with_cluster(entry.cluster.clone());
    let run = |command: Command, ctx: DispatchContext| {
        let bus = bus.clone();
        async move {
            let Outcome::NeedsConfirmation(asked) = bus
                .dispatch(command.clone(), ctx.clone())
                .await
                .expect("asks first")
            else {
                panic!("a scale asks for a confirmation");
            };
            bus.dispatch(
                command,
                ctx.with_confirmation(Confirmation::simple(asked.token)),
            )
            .await
        }
    };

    // --- the plan: what the dialog starts with ------------------------------------------------
    let plan = service.plan_scale(&web).await.expect("plan");
    assert_eq!((plan.desired, plan.autoscaler), (1, None));

    // --- scale up: the subresource is patched and the cluster follows -------------------------
    let outcome = run(scale(2), ctx()).await.expect("scale to 2");
    let Outcome::Completed(output) = outcome else {
        panic!("expected completion");
    };
    assert_eq!(
        output.message.as_deref(),
        Some(format!("Scaled Deployment {}/web from 1 to 2 replicas", ns.name()).as_str())
    );
    assert_eq!(spec_replicas(&api, "web").await, 2);
    crate::eventually("two ready replicas", String::new, || async {
        ready(&api, "web").await == 2
    })
    .await;
    // The Deployment's own spec was not rewritten by anything else: only `replicas` changed.
    let live = api.get("web").await.unwrap();
    assert_eq!(
        live.spec.unwrap().template.spec.unwrap().containers[0]
            .image
            .as_deref(),
        Some(BUSYBOX)
    );

    // --- a dry-run dispatch changes nothing ---------------------------------------------------
    let dry = run(scale(7), ctx().with_dry_run(true))
        .await
        .expect("dry run");
    let Outcome::Completed(output) = dry else {
        panic!("expected completion");
    };
    assert!(output.message.unwrap().starts_with("Dry run: would scale"));
    assert_eq!(
        spec_replicas(&api, "web").await,
        2,
        "a dry run scales nothing"
    );

    // --- zero is accepted and the pods go -----------------------------------------------------
    run(scale(0), ctx()).await.expect("scale to 0");
    assert_eq!(spec_replicas(&api, "web").await, 0);
    crate::eventually("no ready replicas", String::new, || async {
        ready(&api, "web").await == 0
    })
    .await;

    // --- the audit trail ----------------------------------------------------------------------
    let audit = state.audit_log();
    let details: Vec<_> = audit
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Succeeded)
        .map(|r| r.detail.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        details,
        [
            "replicas=2 before=1 after=2",
            "replicas=7 before=2 after=7",
            "replicas=0 before=2 after=0"
        ],
        "{audit:?}"
    );
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));

    // --- read-only: refused, untouched --------------------------------------------------------
    manager
        .set_read_only(&entry.cluster, true)
        .expect("read-only on");
    let error = bus
        .dispatch(scale(3), ctx())
        .await
        .expect_err("read-only refuses");
    assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    assert_eq!(spec_replicas(&api, "web").await, 0);
}
