//! Restart and set image (E12-S05) against the real adapter: `workload::Restart` and
//! `workload::SetImage` through the `CommandBus` and `MutationGuard` on a kind cluster, no UI.
//!
//! * a Deployment restarts: the pod template gets `kubectl.kubernetes.io/restartedAt` and the
//!   controller rolls a new ReplicaSet with a ready pod; a dispatch that is a dry run changes
//!   nothing; the audit record carries the time and the object's version;
//! * the strategic merge patch of `workload::SetImage` works for a Deployment, a StatefulSet and a
//!   DaemonSet: only the named container (and an init container) change, every other container and
//!   field is left alone, and a restart patches the template of each of the three kinds;
//! * a read-only session refuses both and the objects are untouched.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace; nothing touches the node. The
//! StatefulSet has no replicas and the DaemonSet selects a node label nothing has, so their
//! templates are patched without any pod being scheduled.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use kube::Api;
use kube::api::{ListParams, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::workloads::{WorkloadService, register_commands};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, ImageChange};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::images::{PAUSE, PAUSE_PREVIOUS};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::{Value, json};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

/// A bus with the real workload handlers over a session on the kind cluster.
struct Rig {
    bus: CommandBus,
    manager: ClusterSessionManager,
    state: Arc<FakeStatePort>,
    cluster: ClusterId,
}

impl Rig {
    async fn new(kind: &Kind) -> Self {
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
        let mut registry = CommandRegistry::new();
        let handler_sessions = manager.clone();
        registry
            .install("oxikube_app::workloads", move |r| {
                register_commands(r, handler_sessions)
            })
            .expect("register the workload commands");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        Self {
            bus,
            manager,
            state,
            cluster: entry.cluster,
        }
    }

    fn target(&self, kind: &str, namespace: &str, name: &str) -> ResourceRef {
        ResourceRef::namespaced(
            self.cluster.clone(),
            Gvk::new("apps", "v1", kind),
            namespace,
            name,
        )
    }

    fn ctx(&self) -> DispatchContext {
        DispatchContext::new(Initiator::Ui, "tester").with_cluster(self.cluster.clone())
    }

    /// Dispatches `command`, answering the one confirmation as the UI does.
    async fn run(&self, command: Command, ctx: DispatchContext) -> Result<Outcome, DispatchError> {
        // A request the server refuses to dry-run is refused before anything is asked
        // (E19-S07).
        let Outcome::NeedsConfirmation(asked) =
            self.bus.dispatch(command.clone(), ctx.clone()).await?
        else {
            panic!("a rollout command asks for a confirmation");
        };
        self.bus
            .dispatch(
                command,
                ctx.with_confirmation(Confirmation::simple(asked.token)),
            )
            .await
    }
}

fn template(app: &str, with_init: bool) -> Value {
    let mut spec = json!({
        "terminationGracePeriodSeconds": 0,
        "containers": [
            {"name": "main", "image": PAUSE},
            {"name": "side", "image": PAUSE},
        ],
    });
    if with_init {
        spec["initContainers"] = json!([{"name": "setup", "image": PAUSE, "command": ["true"]}]);
    }
    json!({"metadata": {"labels": {"app": app}}, "spec": spec})
}

fn deployment(name: &str, replicas: i32, with_init: bool) -> Deployment {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": name},
        "spec": {"replicas": replicas, "selector": {"matchLabels": {"app": name}},
                 "template": template(name, with_init)}
    }))
    .expect("a deployment")
}

fn statefulset(name: &str) -> StatefulSet {
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": name},
        "spec": {"replicas": 0, "serviceName": name,
                 "selector": {"matchLabels": {"app": name}},
                 "template": template(name, true)}
    }))
    .expect("a statefulset")
}

fn daemonset(name: &str) -> DaemonSet {
    let mut template = template(name, true);
    // A node label nothing has: the DaemonSet controller schedules no pod.
    template["spec"]["nodeSelector"] = json!({"oxikube.test/none": "true"});
    serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "DaemonSet",
        "metadata": {"name": name},
        "spec": {"selector": {"matchLabels": {"app": name}}, "template": template}
    }))
    .expect("a daemonset")
}

fn image_of(containers: &[k8s_openapi::api::core::v1::Container], name: &str) -> String {
    containers
        .iter()
        .find(|c| c.name == name)
        .and_then(|c| c.image.clone())
        .unwrap_or_default()
}

fn annotation(template: &k8s_openapi::api::core::v1::PodTemplateSpec) -> Option<String> {
    template
        .metadata
        .as_ref()?
        .annotations
        .as_ref()?
        .get(RESTARTED_AT)
        .cloned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restarting_a_deployment_rolls_a_new_replicaset() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let deployments = Api::<Deployment>::namespaced(admin.clone(), ns.name());
    let replicasets = Api::<ReplicaSet>::namespaced(admin.clone(), ns.name());
    deployments
        .create(&PostParams::default(), &deployment("web", 1, false))
        .await
        .expect("create");
    let rig = Rig::new(&kind).await;
    let web = rig.target("Deployment", ns.name(), "web");
    let restart = Command::WorkloadRestart {
        target: web.clone(),
    };
    crate::eventually("the first pod ready", String::new, || async {
        deployments
            .get("web")
            .await
            .ok()
            .and_then(|d| d.status?.ready_replicas)
            == Some(1)
    })
    .await;

    // A dry-run dispatch rehearses the patch and changes nothing.
    let dry = rig
        .run(restart.clone(), rig.ctx().with_dry_run(true))
        .await
        .expect("dry run");
    let Outcome::Completed(output) = dry else {
        panic!("expected completion");
    };
    assert!(output.message.unwrap().starts_with("Dry run:"));
    let live = deployments.get("web").await.unwrap();
    assert_eq!(annotation(&live.spec.unwrap().template), None);

    // The restart stamps the template and the controller rolls a second ReplicaSet.
    let Outcome::Completed(output) = rig.run(restart, rig.ctx()).await.expect("restart") else {
        panic!("expected completion");
    };
    assert_eq!(
        output.message.as_deref(),
        Some(
            format!(
                "Restarting Deployment {}/web: new pods are rolling out",
                ns.name()
            )
            .as_str()
        )
    );
    let stamped = annotation(&deployments.get("web").await.unwrap().spec.unwrap().template)
        .expect("restartedAt is on the pod template");
    assert_eq!(stamped, output.data.unwrap()["restarted_at"]);
    crate::eventually("a new ReplicaSet with a ready pod", String::new, || async {
        let sets = replicasets
            .list(&ListParams::default())
            .await
            .unwrap()
            .items;
        let ready: i32 = sets
            .iter()
            .filter(|rs| rs.spec.as_ref().and_then(|s| s.replicas) == Some(1))
            .filter_map(|rs| rs.status.as_ref()?.ready_replicas)
            .sum();
        sets.len() == 2 && ready == 1
    })
    .await;

    // The audit record: the time and the object's version after the patch.
    let audit = rig.state.audit_log();
    let succeeded: Vec<_> = audit
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Succeeded)
        .collect();
    assert_eq!(succeeded.len(), 2, "{audit:?}");
    let detail = succeeded[1].detail.as_deref().unwrap_or_default();
    assert!(
        detail.starts_with(&format!("restartedAt={stamped} rv=")),
        "{detail}"
    );
    assert!(audit.iter().all(|r| r.initiator == Initiator::Ui));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_strategic_patch_sets_only_the_named_containers_of_all_three_kinds() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let deployments = Api::<Deployment>::namespaced(admin.clone(), ns.name());
    let statefulsets = Api::<StatefulSet>::namespaced(admin.clone(), ns.name());
    let daemonsets = Api::<DaemonSet>::namespaced(admin.clone(), ns.name());
    // The Deployment has no replicas, so its pod template is patched without a pod.
    deployments
        .create(&PostParams::default(), &deployment("web", 0, true))
        .await
        .expect("create the deployment");
    statefulsets
        .create(&PostParams::default(), &statefulset("db"))
        .await
        .expect("create the statefulset");
    daemonsets
        .create(&PostParams::default(), &daemonset("agent"))
        .await
        .expect("create the daemonset");
    let rig = Rig::new(&kind).await;

    let change = |kind_name: &str, name: &str, images: &[(&str, &str)]| Command::WorkloadSetImage {
        target: rig.target(kind_name, ns.name(), name),
        images: images
            .iter()
            .map(|(container, image)| ImageChange::new(*container, *image))
            .collect(),
    };
    for (kind_name, name) in [
        ("Deployment", "web"),
        ("StatefulSet", "db"),
        ("DaemonSet", "agent"),
    ] {
        // The plan lists the init container first, then the containers.
        let service = WorkloadService::new(rig.manager.clone());
        let plan = service
            .plan_set_image(&rig.target(kind_name, ns.name(), name))
            .await
            .expect("plan");
        let listed: Vec<_> = plan.containers.iter().map(|c| (&*c.name, c.init)).collect();
        assert_eq!(
            listed,
            [("setup", true), ("main", false), ("side", false)],
            "{kind_name}"
        );

        // One container and the init container change; the other container is not in the patch.
        let done = rig
            .run(
                change(
                    kind_name,
                    name,
                    &[("main", PAUSE_PREVIOUS), ("setup", PAUSE_PREVIOUS)],
                ),
                rig.ctx(),
            )
            .await
            .unwrap_or_else(|error| panic!("{kind_name}: {error}"));
        let Outcome::Completed(output) = done else {
            panic!("expected completion");
        };
        assert!(
            output.message.unwrap().starts_with("Set the image of"),
            "{kind_name}"
        );
        let (containers, inits) = match kind_name {
            "Deployment" => pod_spec(deployments.get(name).await.unwrap().spec.unwrap().template),
            "StatefulSet" => pod_spec(statefulsets.get(name).await.unwrap().spec.unwrap().template),
            _ => pod_spec(daemonsets.get(name).await.unwrap().spec.unwrap().template),
        };
        assert_eq!(image_of(&containers, "main"), PAUSE_PREVIOUS, "{kind_name}");
        assert_eq!(image_of(&inits, "setup"), PAUSE_PREVIOUS, "{kind_name}");
        assert_eq!(
            image_of(&containers, "side"),
            PAUSE,
            "{kind_name}: untouched"
        );
        assert_eq!(
            containers.len(),
            2,
            "{kind_name}: no container was added or dropped"
        );

        // A restart patches the template of the same kind.
        rig.run(
            Command::WorkloadRestart {
                target: rig.target(kind_name, ns.name(), name),
            },
            rig.ctx(),
        )
        .await
        .unwrap_or_else(|error| panic!("{kind_name}: {error}"));
        let template = match kind_name {
            "Deployment" => deployments.get(name).await.unwrap().spec.unwrap().template,
            "StatefulSet" => statefulsets.get(name).await.unwrap().spec.unwrap().template,
            _ => daemonsets.get(name).await.unwrap().spec.unwrap().template,
        };
        assert!(annotation(&template).is_some(), "{kind_name}: restartedAt");
        // ... and left the images the set-image just wrote.
        let (containers, _) = pod_spec(template);
        assert_eq!(image_of(&containers, "main"), PAUSE_PREVIOUS, "{kind_name}");
    }

    // Bad input is refused by the handler before anything is sent.
    let error = rig
        .run(change("Deployment", "web", &[("ghost", PAUSE)]), rig.ctx())
        .await
        .expect_err("an unknown container");
    assert!(
        error.to_string().contains("no container named ghost"),
        "{error}"
    );

    // The audit trail names the old and new image of each set-image.
    let audit = rig.state.audit_log();
    let details: Vec<_> = audit
        .iter()
        // The commits' records: the guard's dry runs before it asks (E19-S07) are their own.
        .filter(|r| {
            &*r.cmd == "workload::SetImage" && r.outcome == AuditOutcome::Succeeded && !r.dry_run
        })
        .map(|r| r.detail.clone().unwrap_or_default())
        .collect();
    assert_eq!(details.len(), 3, "{audit:?}");
    for detail in &details {
        assert!(
            detail.contains(&format!("main={PAUSE}->{PAUSE_PREVIOUS}"))
                && detail.contains(&format!("setup={PAUSE}->{PAUSE_PREVIOUS}"))
                && detail.contains(" rv="),
            "{detail}"
        );
    }

    // Read-only: both refused, the objects untouched.
    rig.manager
        .set_read_only(&rig.cluster, true)
        .expect("read-only on");
    for command in [
        Command::WorkloadRestart {
            target: rig.target("Deployment", ns.name(), "web"),
        },
        change("Deployment", "web", &[("main", PAUSE)]),
    ] {
        let error = rig
            .bus
            .dispatch(command, rig.ctx())
            .await
            .expect_err("read-only refuses");
        assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    }
    let (containers, _) = pod_spec(deployments.get("web").await.unwrap().spec.unwrap().template);
    assert_eq!(image_of(&containers, "main"), PAUSE_PREVIOUS);
}

fn pod_spec(
    template: k8s_openapi::api::core::v1::PodTemplateSpec,
) -> (
    Vec<k8s_openapi::api::core::v1::Container>,
    Vec<k8s_openapi::api::core::v1::Container>,
) {
    let spec = template.spec.expect("a pod spec");
    (spec.containers, spec.init_containers.unwrap_or_default())
}
