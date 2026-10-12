//! Pods grouped by the top of their owner chains: Pod to ReplicaSet to Deployment, Job to
//! CronJob, a lone Pod, and what a missing owner or a cached one costs.

use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::view::RefKind;
use oxikube_testkit::{FakeResourcePort, ResourceCall};
use serde_json::json;

use super::{CLUSTER, owned_by, pod_cm, ready};
use crate::references::{OwnerCache, ReferenceIndex, UsedBy, WorkloadUse, used_by};
use crate::testing::id;

/// A namespaced object of `api_version`/`kind`, optionally controlled by `owner`.
fn object(
    api_version: &str,
    kind: &str,
    name: &str,
    owner: Option<(&str, &str, &str)>,
) -> Resource {
    let mut json = json!({
        "apiVersion": api_version, "kind": kind,
        "metadata": {"name": name, "namespace": "shop", "uid": format!("uid-{name}")},
    });
    if let Some((api, kind, name)) = owner {
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": api, "kind": kind, "name": name,
            "uid": format!("uid-{name}"), "controller": true,
        }]);
    }
    Resource::from_json(json).expect("a resource")
}

fn cluster() -> ClusterId {
    id(CLUSTER)
}

fn target(api_version: &str, kind: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::from_api_version(api_version, kind),
        "shop",
        name,
    )
}

async fn ask(
    index: &ReferenceIndex,
    cache: &OwnerCache,
    reader: &FakeResourcePort,
    cm: &str,
) -> UsedBy {
    used_by(
        index,
        cache,
        reader,
        &cluster(),
        RefKind::ConfigMap,
        Some("shop"),
        cm,
    )
    .await
}

fn workloads(answer: UsedBy) -> Vec<WorkloadUse> {
    match answer {
        UsedBy::Known { workloads, .. } => workloads,
        other => panic!("{other:?}"),
    }
}

fn gets(reader: &FakeResourcePort) -> usize {
    reader
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, ResourceCall::GetOpt { .. } | ResourceCall::Get { .. }))
        .count()
}

#[test]
fn pods_are_grouped_by_the_deployment_that_owns_their_replica_set() {
    let reader = FakeResourcePort::new().with_objects([
        object(
            "apps/v1",
            "ReplicaSet",
            "web-5d",
            Some(("apps/v1", "Deployment", "web")),
        ),
        object(
            "apps/v1",
            "ReplicaSet",
            "web-4c",
            Some(("apps/v1", "Deployment", "web")),
        ),
        object("apps/v1", "Deployment", "web", None),
        object(
            "apps/v1",
            "ReplicaSet",
            "api-7f",
            Some(("apps/v1", "Deployment", "api")),
        ),
        object("apps/v1", "Deployment", "api", None),
    ]);
    let rs =
        |pod: &str, rs: &str| owned_by(pod_cm(pod, "shop", "cfg"), "apps/v1", "ReplicaSet", rs);
    let index = ready(vec![
        rs("web-5d-a", "web-5d"),
        rs("web-5d-b", "web-5d"),
        rs("web-4c-a", "web-4c"),
        rs("api-7f-a", "api-7f"),
    ]);
    let cache = OwnerCache::default();
    let answer = futures::executor::block_on(ask(&index, &cache, &reader, "cfg"));
    let UsedBy::Known {
        workloads,
        pods,
        partial,
    } = answer
    else {
        panic!("known");
    };
    assert_eq!(pods, 4);
    assert!(!partial);
    assert_eq!(
        workloads,
        [
            WorkloadUse {
                target: target("apps/v1", "Deployment", "api"),
                pods: 1
            },
            WorkloadUse {
                target: target("apps/v1", "Deployment", "web"),
                pods: 3
            },
        ],
        "the old and the new replica set roll up into one Deployment"
    );
}

#[test]
fn each_distinct_owner_is_read_once_and_remembered() {
    let reader = FakeResourcePort::new().with_objects([
        object(
            "apps/v1",
            "ReplicaSet",
            "web-5d",
            Some(("apps/v1", "Deployment", "web")),
        ),
        object("apps/v1", "Deployment", "web", None),
    ]);
    let pods: Vec<Resource> = (0..20)
        .map(|i| {
            owned_by(
                pod_cm(&format!("web-{i:02}"), "shop", "cfg"),
                "apps/v1",
                "ReplicaSet",
                "web-5d",
            )
        })
        .collect();
    let index = ready(pods);
    let cache = OwnerCache::default();
    let first = futures::executor::block_on(ask(&index, &cache, &reader, "cfg"));
    assert_eq!(workloads(first)[0].pods, 20);
    assert_eq!(
        gets(&reader),
        2,
        "the replica set and the deployment, once, for twenty pods"
    );
    assert_eq!(cache.len(), 1);

    let again = futures::executor::block_on(ask(&index, &cache, &reader, "cfg"));
    assert_eq!(workloads(again)[0].pods, 20);
    assert_eq!(gets(&reader), 2, "the second read is served from the cache");
}

#[test]
fn a_cron_job_run_rolls_up_to_the_cron_job() {
    let reader = FakeResourcePort::new().with_objects([
        object(
            "batch/v1",
            "Job",
            "nightly-2900",
            Some(("batch/v1", "CronJob", "nightly")),
        ),
        object("batch/v1", "CronJob", "nightly", None),
    ]);
    let index = ready(vec![owned_by(
        pod_cm("nightly-2900-x", "shop", "cfg"),
        "batch/v1",
        "Job",
        "nightly-2900",
    )]);
    let answer = futures::executor::block_on(ask(&index, &OwnerCache::default(), &reader, "cfg"));
    assert_eq!(
        workloads(answer),
        [WorkloadUse {
            target: target("batch/v1", "CronJob", "nightly"),
            pods: 1
        }]
    );
}

#[test]
fn unowned_and_node_mirrored_pods_list_themselves() {
    let reader = FakeResourcePort::new();
    let mirror = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "etcd-node1", "namespace": "shop", "uid": "u",
            "ownerReferences": [{"apiVersion": "v1", "kind": "Node", "name": "node1", "uid": "n", "controller": true}]},
        "spec": {"volumes": [{"name": "v", "configMap": {"name": "cfg"}}]},
        "status": {"phase": "Running"},
    });
    let index = ready(vec![
        pod_cm("lone", "shop", "cfg"),
        Resource::from_json(mirror).unwrap(),
    ]);
    let answer = futures::executor::block_on(ask(&index, &OwnerCache::default(), &reader, "cfg"));
    assert_eq!(
        workloads(answer),
        [
            WorkloadUse {
                target: target("v1", "Pod", "etcd-node1"),
                pods: 1
            },
            WorkloadUse {
                target: target("v1", "Pod", "lone"),
                pods: 1
            },
        ]
    );
    assert_eq!(gets(&reader), 0, "no owner to read");
}

#[test]
fn an_owner_that_is_gone_is_listed_as_it_is() {
    // The replica set cannot be read (deleted, or not allowed): the pods are still attributed to it.
    let reader = FakeResourcePort::new();
    let index = ready(vec![owned_by(
        pod_cm("web-5d-a", "shop", "cfg"),
        "apps/v1",
        "ReplicaSet",
        "web-5d",
    )]);
    let cache = OwnerCache::default();
    let answer = futures::executor::block_on(ask(&index, &cache, &reader, "cfg"));
    assert_eq!(
        workloads(answer),
        [WorkloadUse {
            target: target("apps/v1", "ReplicaSet", "web-5d"),
            pods: 1
        }]
    );
    assert!(cache.is_empty(), "a failed read is not remembered");
}

#[test]
fn an_unknown_answer_is_passed_on_and_an_unused_object_is_an_empty_list() {
    let reader = FakeResourcePort::new();
    let cache = OwnerCache::default();
    let warming = ReferenceIndex::new();
    assert_eq!(
        futures::executor::block_on(ask(&warming, &cache, &reader, "cfg")),
        UsedBy::Warming
    );

    let index = ready(vec![pod_cm("a", "shop", "cfg")]);
    let none = futures::executor::block_on(ask(&index, &cache, &reader, "nothing"));
    assert_eq!(
        none,
        UsedBy::Known {
            workloads: vec![],
            pods: 0,
            partial: false
        }
    );
}
