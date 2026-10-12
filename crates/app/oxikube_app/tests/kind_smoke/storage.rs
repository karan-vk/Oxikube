//! Storage actions (E12-S15) against the real adapter: `pvc::Resize`, `storageclass::SetDefault`
//! and the claims' "used by" index through the `CommandBus`, `MutationGuard` and `ResourceStore`
//! on a kind cluster, no UI.
//!
//! * the resize flow reads a claim's StorageClass from the API server: a class that does not allow
//!   expansion blocks (kind's own `standard` class is checked against what the cluster says), one
//!   that does lets the resize through to the server, whose dry run refuses a claim that is not
//!   bound yet; nothing is patched and the audit record says `Failed` with both sizes;
//! * `storageclass::SetDefault` as a dispatch that is itself a dry run: the server accepts the
//!   merge patches of the new and of the previous default and the cluster's default is unchanged
//!   (the cluster is shared with every other suite, so the default is never really moved);
//! * the "used by" index follows a Pod that mounts a claim in the test's namespace.
//!
//! Every namespaced object lives in the test's own `oxi-test-<rand>` namespace; the two cluster
//! scoped StorageClasses carry the namespace's name and are deleted when the test ends, also when
//! it fails (`ClassGuard`, as `TestNamespace` does for the namespace).

use std::sync::Arc;

use k8s_openapi::api::core::v1::{
    Container, PersistentVolumeClaim, PersistentVolumeClaimSpec, PersistentVolumeClaimVolumeSource,
    Pod, PodSpec, Volume, VolumeResourceRequirements,
};
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::Api;
use kube::api::{DeleteParams, ObjectMeta, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use oxikube_app::{
    CommandBus, DefaultClassFlow, MutationGuard, ReferenceFeed, ResizeFlow, UsageStatus,
};
use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::view::RefKind;
use oxikube_domain::view::storage::{Expansion, ResizeBlock};
use oxikube_kube::{ConnectorConfig, KubeConnector, KubeDiscovery, KubeResources, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort, images};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Deletes cluster-scoped StorageClasses when dropped (also on panic), best effort: a test that
/// fails half way must not leave classes on the shared cluster.
struct ClassGuard {
    context: String,
    names: Vec<String>,
}

impl Drop for ClassGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kubectl")
            .arg("--context")
            .arg(&self.context)
            .args([
                "delete",
                "storageclass",
                "--ignore-not-found",
                "--wait=false",
            ])
            .args(&self.names)
            .output();
    }
}

fn class(name: &str, expandable: bool) -> StorageClass {
    StorageClass {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            ..ObjectMeta::default()
        },
        // No provisioner of this name runs, so a claim of the class stays Pending and nothing is
        // ever created on the node.
        provisioner: "oxikube.example/none".to_owned(),
        allow_volume_expansion: Some(expandable),
        volume_binding_mode: Some("WaitForFirstConsumer".to_owned()),
        ..StorageClass::default()
    }
}

fn claim(namespace: &str, name: &str, class: &str) -> PersistentVolumeClaim {
    PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            ..ObjectMeta::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
            storage_class_name: Some(class.to_owned()),
            resources: Some(VolumeResourceRequirements {
                requests: Some([("storage".to_owned(), Quantity("1Gi".to_owned()))].into()),
                ..VolumeResourceRequirements::default()
            }),
            ..PersistentVolumeClaimSpec::default()
        }),
        ..PersistentVolumeClaim::default()
    }
}

fn pod_mounting(namespace: &str, name: &str, claim: &str) -> Pod {
    Pod {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            containers: vec![Container {
                name: "main".to_owned(),
                image: Some(images::BUSYBOX.to_owned()),
                command: Some(vec!["sleep".to_owned(), "3600".to_owned()]),
                ..Container::default()
            }],
            volumes: Some(vec![Volume {
                name: "data".to_owned(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: claim.to_owned(),
                    read_only: None,
                }),
                ..Volume::default()
            }]),
            termination_grace_period_seconds: Some(0),
            ..PodSpec::default()
        }),
        ..Pod::default()
    }
}

async fn default_classes(api: &Api<StorageClass>) -> Vec<String> {
    let mut names: Vec<String> = api
        .list(&Default::default())
        .await
        .expect("list storage classes")
        .items
        .into_iter()
        .filter(|c| {
            c.metadata.annotations.as_ref().is_some_and(|a| {
                [
                    "storageclass.kubernetes.io/is-default-class",
                    "storageclass.beta.kubernetes.io/is-default-class",
                ]
                .iter()
                .any(|k| a.get(*k).is_some_and(|v| v == "true"))
            })
        })
        .filter_map(|c| c.metadata.name)
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resize_set_default_and_used_by_against_a_kind_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let (growable, fixed) = (
        format!("{}-grow", ns.name()),
        format!("{}-fixed", ns.name()),
    );
    // Armed before the classes exist: deleting a class that was never created is a no-op.
    let _class_guard = ClassGuard {
        context: kind.context.to_string(),
        names: vec![growable.clone(), fixed.clone()],
    };
    let classes = Api::<StorageClass>::all(admin_client.clone());
    let claims = Api::<PersistentVolumeClaim>::namespaced(admin_client.clone(), ns.name());
    let pods = Api::<Pod>::namespaced(admin_client.clone(), ns.name());
    classes
        .create(&PostParams::default(), &class(&growable, true))
        .await
        .expect("create a growable class");
    classes
        .create(&PostParams::default(), &class(&fixed, false))
        .await
        .expect("create a fixed class");
    claims
        .create(&PostParams::default(), &claim(ns.name(), "data", &growable))
        .await
        .expect("create a claim");
    claims
        .create(&PostParams::default(), &claim(ns.name(), "frozen", &fixed))
        .await
        .expect("create a claim of the fixed class");
    pods.create(
        &PostParams::default(),
        &pod_mounting(ns.name(), "user", "data"),
    )
    .await
    .expect("create a pod");

    let entry = catalog_entry(&kind.context);
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
    manager.open(&entry, SessionOptions::default());
    manager.connect(&entry.cluster).await.expect("connect");
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::storage", |r| {
            oxikube_app::storage::register_commands(r, manager.clone())
        })
        .expect("register the storage commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let claim_ref = |name: &str| {
        ResourceRef::namespaced(
            entry.cluster.clone(),
            Gvk::new("", "v1", "PersistentVolumeClaim"),
            ns.name(),
            name,
        )
    };

    // --- resize: the class the API server reports decides ------------------------------------
    let resize = ResizeFlow::new(bus.clone(), manager.clone(), "tester");
    let frozen = resize
        .open(&claim_ref("frozen"))
        .await
        .expect("open frozen");
    assert_eq!(
        frozen.blocked(),
        Some(ResizeBlock::ExpansionDisallowed {
            class: fixed.as_str().into()
        }),
        "a class without allowVolumeExpansion blocks the resize"
    );
    assert!(frozen.check("5Gi").is_err());

    // kind's own default class, whichever way this cluster is set up, is reported as it is.
    let standard = classes.get_opt("standard").await.expect("get standard");
    if let Some(standard) = standard {
        let allows = standard.allow_volume_expansion == Some(true);
        let mut std_claim = claim(ns.name(), "std", "standard");
        std_claim.spec.as_mut().unwrap().storage_class_name = Some("standard".to_owned());
        claims
            .create(&PostParams::default(), &std_claim)
            .await
            .expect("create a claim of the default class");
        let std = resize.open(&claim_ref("std")).await.expect("open std");
        assert_eq!(
            matches!(std.expansion, Expansion::Allowed),
            allows,
            "the flow follows the class: {:?}",
            std.expansion
        );
    }

    let data = resize.open(&claim_ref("data")).await.expect("open data");
    assert_eq!(data.expansion, Expansion::Allowed);
    assert_eq!(data.current().unwrap().to_string(), "1Gi");
    assert!(matches!(
        data.check("1Gi"),
        Err(ResizeBlock::NotLarger { .. })
    ));
    let bigger = data.check("2Gi").expect("2Gi grows the claim");
    // The claim is not bound (nothing provisions it), so the API server's admission refuses the
    // rehearsal: the real patch is never sent and the claim stays at 1Gi.
    let error = resize
        .run(&data, bigger)
        .await
        .expect_err("a claim that is not bound cannot be expanded");
    assert!(
        matches!(error.kind(), ErrorKind::Forbidden | ErrorKind::Validation),
        "{error:?}"
    );
    assert!(
        error.message().to_lowercase().contains("bound"),
        "the server's own reason: {}",
        error.message()
    );
    let live = claims.get("data").await.expect("get the claim");
    let requested = live.spec.unwrap().resources.unwrap().requests.unwrap();
    assert_eq!(requested["storage"].0, "1Gi", "nothing was patched");
    let audit = state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "pvc::Resize");
    assert_eq!(audit[0].outcome, AuditOutcome::Failed);
    assert_eq!(audit[0].detail.as_deref(), Some("from=1Gi to=2Gi"));

    // A read-only session refuses, for the user and for an agent alike.
    manager.set_read_only(&entry.cluster, true).unwrap();
    let denied = resize
        .run(&data, data.check("3Gi").unwrap())
        .await
        .expect_err("read-only");
    assert_eq!(denied.kind(), ErrorKind::Forbidden);
    manager.set_read_only(&entry.cluster, false).unwrap();

    // --- set default, as a dry run: accepted by the server, the cluster unchanged ---------------
    let before = default_classes(&classes).await;
    let class_ref = ResourceRef::cluster_scoped(
        entry.cluster.clone(),
        Gvk::new("storage.k8s.io", "v1", "StorageClass"),
        growable.as_str(),
    );
    let default_flow = DefaultClassFlow::new(bus.clone(), manager.clone(), "tester");
    let context = default_flow.open(&class_ref).await.expect("open the class");
    assert_eq!(
        context
            .previous
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>(),
        before,
        "the previous defaults are the cluster's own"
    );
    let command = Command::StorageClassSetDefault {
        target: class_ref.clone(),
        clear_previous: true,
    };
    let ctx = DispatchContext::new(Initiator::Command, "tester").with_dry_run(true);
    let Ok(Outcome::NeedsConfirmation(request)) = bus.dispatch(command.clone(), ctx.clone()).await
    else {
        panic!("expected a confirmation request");
    };
    let done = bus
        .dispatch(
            command,
            ctx.with_confirmation(Confirmation::simple(request.token)),
        )
        .await
        .expect("the server accepts the dry-run patches");
    let Outcome::Completed(output) = done else {
        panic!("completed");
    };
    assert_eq!(output.data.unwrap()["outcome"], "dry_run");
    assert_eq!(
        default_classes(&classes).await,
        before,
        "a dry run moves nothing"
    );

    // --- used by: a pod in this namespace mounts `data` -----------------------------------------
    manager
        .set_namespace_selection(&entry.cluster, NamespaceSelection::single(ns.name()))
        .unwrap();
    let session = manager.get(&entry.cluster).expect("the session");
    let reader = Arc::new(KubeResources::new(
        admin_client.clone(),
        KubeDiscovery::new(admin_client.clone()),
    ));
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    let store = ResourceStore::new(
        session.id().clone(),
        StorePorts {
            resources: reader.clone(),
            tables: reader,
        },
        StoreRuntime {
            spawner,
            clock: Arc::new(TokioClock),
            probe: None,
        },
        StoreOptions::default(),
    );
    let feed = ReferenceFeed::start(&store, &manager);
    let index = feed.index().clone();
    crate::eventually(
        "the pod to be counted",
        || format!("{index:?}"),
        || async {
            index.count(RefKind::PersistentVolumeClaim, Some(ns.name()), "data") == Some(1)
        },
    )
    .await;
    assert_eq!(index.status(), UsageStatus::Ready);
    assert_eq!(
        index.count(RefKind::PersistentVolumeClaim, Some(ns.name()), "frozen"),
        Some(0)
    );
    let oxikube_app::Usage::Pods { pods: users, .. } =
        index.usage(RefKind::PersistentVolumeClaim, Some(ns.name()), "data")
    else {
        panic!("the pods are known");
    };
    assert_eq!(
        users
            .iter()
            .map(|p| p.pod.name.to_string())
            .collect::<Vec<_>>(),
        ["user"]
    );
    pods.delete("user", &DeleteParams::default().grace_period(0))
        .await
        .expect("delete the pod");
    crate::eventually(
        "the pod to be forgotten",
        || format!("{index:?}"),
        || async {
            index.count(RefKind::PersistentVolumeClaim, Some(ns.name()), "data") == Some(0)
        },
    )
    .await;
    drop(feed);
}
