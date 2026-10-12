//! Pod actions (E12-S03) against the real adapter: `pod::Evict`, `pod::Delete`,
//! `pod::ForceDelete` and `pod::Sanitize` through the `CommandBus` and `MutationGuard` on a kind
//! cluster, no UI, plus the reads of the pod panel (owner chain, node, environment sources).
//!
//! * an eviction goes through the `eviction` subresource (the pod leaves), and one a
//!   PodDisruptionBudget forbids is refused with "blocked by disruption budget <name>" and evicts
//!   nothing;
//! * a delete starts the pod's own grace period (it is still there, Terminating) and a force
//!   delete (grace 0, the pod's name typed) takes the object away at once;
//! * a sweep deletes only the Completed and Failed pods, one audit record each under a shared
//!   batch id, and leaves a running pod alone;
//! * a dispatch that is itself a dry run changes nothing, and a read-only session is refused
//!   before any request.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace; evictions and deletes touch
//! only pods of that namespace, and nothing touches a node.

use std::sync::Arc;

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::Api;
use kube::api::{ListParams, ObjectMeta, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::workloads::{WorkloadService, register_commands};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort, images};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn pod_ref(cluster: &ClusterId, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), namespace, name)
}

/// A pod that runs `command` and then stops (`restartPolicy: Never`): Completed for `true`,
/// Failed for `false`. `sleep` ignores SIGTERM as PID 1, so a delete with the default grace period
/// leaves it Terminating for 30 seconds: what a force delete is for.
fn pod(name: &str, command: &[&str], labels: serde_json::Value, grace: Option<i64>) -> Pod {
    serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "labels": labels},
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": grace,
            "containers": [{"name": "run", "image": images::E2E_BUSYBOX, "command": command}],
        },
    }))
    .expect("a valid pod")
}

async fn phase(pods: &Api<Pod>, name: &str) -> Option<String> {
    pods.get_opt(name)
        .await
        .ok()
        .flatten()
        .and_then(|pod| pod.status)
        .and_then(|status| status.phase)
}

/// Dispatches `command` as the UI and answers the confirmation the guard asks for (a typed name
/// when it asks for one).
async fn run(
    bus: &CommandBus,
    command: Command,
    dry_run: bool,
) -> Result<oxikube_app::CommandOutput, DispatchError> {
    let ctx = DispatchContext::new(Initiator::Ui, "tester").with_dry_run(dry_run);
    let request = match bus.dispatch(command.clone(), ctx.clone()).await? {
        Outcome::NeedsConfirmation(request) => request,
        Outcome::Completed(_) => panic!("a pod mutation still asks"),
    };
    let answer = match request.expected_name {
        Some(name) => Confirmation::typed(request.token, name),
        None => Confirmation::simple(request.token),
    };
    bus.dispatch(command, ctx.with_confirmation(answer))
        .await
        .map(|outcome| outcome.completed().expect("the handler ran"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn evict_delete_force_delete_and_sanitize_on_a_kind_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(admin_client.clone(), ns.name());
    let create = |p: Pod| {
        let pods = pods.clone();
        async move {
            pods.create(&PostParams::default(), &p)
                .await
                .expect("create the pod");
        }
    };
    create(pod("done", &["true"], json!({}), Some(0))).await;
    create(pod("broken", &["false"], json!({}), Some(0))).await;
    // The default grace period (30 s): what a delete starts and a force delete cuts short.
    create(pod("sleeper", &["sleep", "3600"], json!({}), None)).await;
    create(pod("victim", &["sleep", "3600"], json!({}), Some(0))).await;
    create(pod(
        "guarded",
        &["sleep", "3600"],
        json!({"app": "guarded"}),
        Some(0),
    ))
    .await;
    for (name, want) in [("done", "Succeeded"), ("broken", "Failed")] {
        crate::eventually(&format!("{name} to be {want}"), String::new, || async {
            phase(&pods, name).await.as_deref() == Some(want)
        })
        .await;
    }
    for name in ["sleeper", "victim", "guarded"] {
        crate::eventually(&format!("{name} to run"), String::new, || async {
            phase(&pods, name).await.as_deref() == Some("Running")
        })
        .await;
    }

    // A budget that keeps `guarded` up: one pod, one required (`minAvailable` works for pods
    // nothing controls; `maxUnavailable` needs a controller to count by).
    let budgets = Api::<PodDisruptionBudget>::namespaced(admin_client.clone(), ns.name());
    let budget: PodDisruptionBudget = serde_json::from_value(json!({
        "apiVersion": "policy/v1", "kind": "PodDisruptionBudget",
        "metadata": {"name": "guard"},
        "spec": {"minAvailable": 1, "selector": {"matchLabels": {"app": "guarded"}}},
    }))
    .unwrap();
    budgets
        .create(&PostParams::default(), &budget)
        .await
        .expect("create the budget");
    crate::eventually("the budget to count the pod", String::new, || async {
        budgets
            .get("guard")
            .await
            .ok()
            .and_then(|b| b.status)
            .is_some_and(|s| s.current_healthy >= Some(1) && s.observed_generation.is_some())
    })
    .await;

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
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::workloads", |r| {
            register_commands(r, manager.clone())
        })
        .expect("register the workload commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let cluster = admin.cluster.clone();
    let at = |name: &str| pod_ref(&cluster, ns.name(), name);

    // --- the budget forbids the eviction, and says which budget ---------------------------------
    let error = run(
        &bus,
        Command::PodEvict {
            target: at("guarded"),
        },
        false,
    )
    .await
    .unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("blocked by disruption budget guard"),
        "{text}"
    );
    assert!(!text.contains("429"), "{text}");
    assert!(
        pods.get_opt("guarded")
            .await
            .unwrap()
            .is_some_and(|p| p.metadata.deletion_timestamp.is_none()),
        "the guarded pod was not evicted"
    );

    // --- a dry-run eviction changes nothing -----------------------------------------------------
    run(
        &bus,
        Command::PodEvict {
            target: at("victim"),
        },
        true,
    )
    .await
    .expect("a dry run");
    assert!(
        pods.get("victim")
            .await
            .unwrap()
            .metadata
            .deletion_timestamp
            .is_none(),
        "a dry run evicts nothing"
    );

    // --- an eviction goes through the subresource -----------------------------------------------
    let output = run(
        &bus,
        Command::PodEvict {
            target: at("victim"),
        },
        false,
    )
    .await
    .expect("evict");
    assert!(output.message.unwrap().starts_with("Evicted Pod"));
    assert!(
        pods.get("victim")
            .await
            .is_err_and(|e| matches!(&e, kube::Error::Api(s) if s.code == 404))
            || pods
                .get("victim")
                .await
                .unwrap()
                .metadata
                .deletion_timestamp
                .is_some(),
        "the pod is leaving"
    );

    // --- a delete starts the grace period; a force delete ends it ------------------------------
    let output = run(
        &bus,
        Command::PodDelete {
            target: at("sleeper"),
            grace_period_seconds: None,
        },
        false,
    )
    .await
    .expect("delete");
    assert_eq!(output.data.unwrap()["outcome"], "deleting");
    let leaving = pods
        .get("sleeper")
        .await
        .expect("still there while it shuts down");
    assert!(leaving.metadata.deletion_timestamp.is_some());
    assert_eq!(leaving.metadata.deletion_grace_period_seconds, Some(30));
    let force = Command::PodForceDelete {
        target: at("sleeper"),
    };
    // The guard asks for the pod's name, and a wrong one does nothing.
    let ctx = DispatchContext::new(Initiator::Ui, "tester");
    let Outcome::NeedsConfirmation(request) =
        bus.dispatch(force.clone(), ctx.clone()).await.unwrap()
    else {
        panic!("a force delete asks");
    };
    assert_eq!(request.expected_name.as_deref(), Some("sleeper"));
    let wrong = bus
        .dispatch(
            force.clone(),
            ctx.with_confirmation(Confirmation::typed(request.token, "other")),
        )
        .await
        .unwrap_err();
    assert!(matches!(wrong, DispatchError::Confirmation(_)), "{wrong:?}");
    run(&bus, force, false).await.expect("force delete");
    crate::eventually(
        "the force-deleted pod to leave the API at once",
        String::new,
        || async { pods.get_opt("sleeper").await.unwrap().is_none() },
    )
    .await;

    // --- a sweep deletes only what finished -------------------------------------------------------
    let sweep = Command::PodSanitize {
        cluster: cluster.clone(),
        namespace: Some(ns.name().to_owned()),
        targets: Vec::new(),
    };
    run(&bus, sweep.clone(), true).await.expect("dry run");
    assert!(
        pods.get_opt("done").await.unwrap().is_some(),
        "a dry run sweeps nothing"
    );
    let output = run(&bus, sweep, false).await.expect("sanitize");
    assert_eq!(output.message.as_deref(), Some("Deleted 2 pods"));
    let batch = output.data.unwrap()["batch"].as_str().unwrap().to_owned();
    crate::eventually("the finished pods to leave", String::new, || async {
        let left: Vec<_> = pods
            .list(&ListParams::default())
            .await
            .unwrap()
            .items
            .into_iter()
            .filter_map(|p| p.metadata.name)
            .collect();
        !left.iter().any(|n| n == "done" || n == "broken")
    })
    .await;
    assert!(
        pods.get_opt("guarded").await.unwrap().is_some(),
        "a running pod is not swept"
    );

    // --- read-only: refused before any request --------------------------------------------------
    manager.set_read_only(&cluster, true).unwrap();
    for command in [
        Command::PodEvict {
            target: at("guarded"),
        },
        Command::PodForceDelete {
            target: at("guarded"),
        },
    ] {
        let error = bus
            .dispatch(command, DispatchContext::new(Initiator::Agent, "agent"))
            .await
            .unwrap_err();
        assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    }
    manager.set_read_only(&cluster, false).unwrap();

    // --- the audit trail ----------------------------------------------------------------------
    let audit = state.audit_log();
    let per_pod: Vec<_> = audit
        .iter()
        .filter(|r| &*r.cmd == "pod::Delete" && !r.dry_run)
        .collect();
    assert!(
        per_pod
            .iter()
            .filter(|r| r.detail.as_deref()
                == Some(format!("batch={batch} via=pod::Sanitize").as_str()))
            .count()
            == 2,
        "one record per swept pod under the batch: {audit:?}"
    );
    assert!(
        audit
            .iter()
            .any(|r| &*r.cmd == "pod::Evict" && r.outcome == AuditOutcome::Failed)
    );
    assert!(
        audit
            .iter()
            .any(|r| &*r.cmd == "pod::ForceDelete" && r.detail.as_deref() == Some("grace=0"))
    );
    assert!(
        audit
            .iter()
            .filter(|r| r.outcome == AuditOutcome::Denied && r.initiator == Initiator::Agent)
            .count()
            == 2
    );
}

/// The reads of the pod panel against the real adapter: the owner chain walks Pod, ReplicaSet,
/// Deployment (scopes from discovery), the node is the pod's `spec.nodeName`, and the environment's
/// ConfigMap and Secret are read once each, a missing one reported rather than failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pod_panels_reads_on_a_kind_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let configmaps = Api::<ConfigMap>::namespaced(admin_client.clone(), ns.name());
    configmaps
        .create(
            &PostParams::default(),
            &ConfigMap {
                metadata: ObjectMeta {
                    name: Some("web-config".to_owned()),
                    ..ObjectMeta::default()
                },
                data: Some([("MODE".to_owned(), "prod".to_owned())].into()),
                ..ConfigMap::default()
            },
        )
        .await
        .expect("create the ConfigMap");
    let deployments =
        Api::<k8s_openapi::api::apps::v1::Deployment>::namespaced(admin_client.clone(), ns.name());
    let deployment: k8s_openapi::api::apps::v1::Deployment = serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web"},
        "spec": {
            "replicas": 1,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {"containers": [{
                    "name": "run", "image": images::E2E_BUSYBOX, "command": ["sleep", "3600"],
                    "envFrom": [{"configMapRef": {"name": "web-config"}}],
                    "env": [{"name": "SECRET", "valueFrom": {"secretKeyRef": {"name": "nobody", "key": "k"}}}],
                }]},
            },
        },
    }))
    .unwrap();
    deployments
        .create(&PostParams::default(), &deployment)
        .await
        .expect("create the Deployment");
    let pods = Api::<Pod>::namespaced(admin_client.clone(), ns.name());
    crate::eventually(
        "the Deployment's pod to be scheduled",
        String::new,
        || async {
            pods.list(&ListParams::default())
                .await
                .unwrap()
                .items
                .iter()
                .any(|p| p.spec.as_ref().and_then(|s| s.node_name.as_ref()).is_some())
        },
    )
    .await;
    let pod = pods
        .list(&ListParams::default())
        .await
        .unwrap()
        .items
        .remove(0);
    let name = pod.metadata.name.clone().unwrap();

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
    let service = WorkloadService::new(manager.clone());
    let target = pod_ref(&admin.cluster, ns.name(), &name);

    let chain = service.owner_chain(&target).await.expect("the chain");
    let described: Vec<_> = chain
        .iter()
        .map(|r| (r.gvk.kind.to_string(), r.name.to_string()))
        .collect();
    assert_eq!(described.len(), 2, "{described:?}");
    assert_eq!(described[0].0, "ReplicaSet");
    assert_eq!(described[1], ("Deployment".to_owned(), "web".to_owned()));
    assert!(chain.iter().all(|r| r.namespace() == Some(ns.name())));
    let owner = service.owner_of(&target).await.unwrap().expect("an owner");
    assert_eq!(owner, chain[0]);
    let node = service.node_of(&target).await.unwrap().expect("a node");
    assert_eq!(&*node.gvk.kind, "Node");
    assert!(node.namespace().is_none());

    let live = manager
        .get(&admin.cluster)
        .and_then(|s| s.resources())
        .expect("a reader")
        .get(&target.gvk, target.namespace(), &target.name)
        .await
        .unwrap();
    let sources = service
        .env_sources(&admin.cluster, &live)
        .await
        .expect("sources");
    use oxikube_domain::view::{PodDetail, SourceKind, SourceState};
    assert!(matches!(
        sources.get(SourceKind::ConfigMap, &Arc::from("web-config")),
        Some(SourceState::Found(_))
    ));
    assert_eq!(
        sources.get(SourceKind::Secret, &Arc::from("nobody")),
        Some(&SourceState::NotFound)
    );
    let rows = PodDetail::from_resource(&live).unwrap().containers[0].env_rows(&live, &sources);
    assert!(rows.iter().any(|r| r.name == "MODE"));
}
