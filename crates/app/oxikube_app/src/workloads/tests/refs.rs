//! What a pod points at: the owner chain, the node, and the ConfigMaps and Secrets of its
//! environment (one `get` each, no feed).

use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_domain::OxiError;
use oxikube_domain::view::{EnvValue, Missing, PodDetail, SourceKind, SourceState, SourceValue};
use oxikube_testkit::ResourceCall;

use super::*;
use crate::workloads::WorkloadService;

fn service(t: &Workloads) -> WorkloadService {
    WorkloadService::new(t.h.manager.clone())
}

fn replica_set() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {"name": "web-5d", "namespace": "default", "ownerReferences": [{
            "apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
            "uid": "dep-uid", "controller": true}]},
    }))
    .unwrap()
}

fn deployment() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "default"},
    }))
    .unwrap()
}

#[test]
fn the_owner_chain_runs_from_the_pod_up_to_its_deployment() {
    let t = Workloads::new(false);
    t.port().insert(pod("web-0", "Running"));
    t.port().insert(replica_set());
    t.port().insert(deployment());
    let chain = service(&t)
        .owner_chain(&pod_ref("web-0"))
        .now_or_never()
        .unwrap()
        .unwrap();
    let described: Vec<_> = chain
        .iter()
        .map(|r| (r.gvk.kind.to_string(), r.name.to_string()))
        .collect();
    assert_eq!(
        described,
        [
            ("ReplicaSet".to_owned(), "web-5d".to_owned()),
            ("Deployment".to_owned(), "web".to_owned())
        ]
    );
    assert!(chain.iter().all(|r| r.namespace() == Some("default")));
    assert!(t.port().mutating_calls().is_empty());
}

#[test]
fn a_chain_ends_at_an_owner_that_is_gone_and_still_lists_it() {
    let t = Workloads::new(false);
    t.port().insert(pod("web-0", "Running"));
    let chain = service(&t)
        .owner_chain(&pod_ref("web-0"))
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(&*chain[0].name, "web-5d");
}

#[test]
fn a_pod_nobody_owns_has_no_owner_and_the_scheduler_names_its_node() {
    let t = Workloads::new(false);
    let orphan = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "lonely", "namespace": "default"},
        "spec": {"nodeName": "worker-2", "containers": []},
    }))
    .unwrap();
    t.port().insert(orphan);
    t.port().insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "pending", "namespace": "default"},
            "spec": {"containers": []},
        }))
        .unwrap(),
    );
    let svc = service(&t);
    assert_eq!(
        svc.owner_of(&pod_ref("lonely"))
            .now_or_never()
            .unwrap()
            .unwrap(),
        None
    );
    let node = svc
        .node_of(&pod_ref("lonely"))
        .now_or_never()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        (&*node.gvk.kind, &*node.name, node.namespace()),
        ("Node", "worker-2", None)
    );
    assert_eq!(
        svc.node_of(&pod_ref("pending"))
            .now_or_never()
            .unwrap()
            .unwrap(),
        None,
        "not scheduled yet"
    );
    let owner = svc.owner_of(&pod_ref("web-0")).now_or_never().unwrap();
    assert!(owner.is_err(), "a pod that is not there cannot be read");
}

fn env_pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "default"},
        "spec": {"containers": [{
            "name": "app",
            "envFrom": [{"configMapRef": {"name": "web-config"}}],
            "env": [
                {"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "web-secret", "key": "token"}}},
                {"name": "OTHER", "valueFrom": {"secretKeyRef": {"name": "web-secret", "key": "other"}}},
                {"name": "GONE", "valueFrom": {"configMapKeyRef": {"name": "missing", "key": "k"}}},
                {"name": "DENIED", "valueFrom": {"secretKeyRef": {"name": "locked", "key": "k"}}},
            ],
        }]},
    }))
    .unwrap()
}

#[test]
fn env_sources_reads_each_object_once_and_never_writes() {
    let t = Workloads::new(false);
    t.port().insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "web-config", "namespace": "default"},
            "data": {"mode": "prod"},
        }))
        .unwrap(),
    );
    t.port().insert(
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "web-secret", "namespace": "default"},
            "data": {"token": "aHVudGVyMg=="},
        }))
        .unwrap(),
    );
    // `get_opt` of `locked` is forbidden; the others read the store.
    let port = t.port();
    let pod = env_pod();
    let svc = service(&t);
    // Script by call order: web-config, web-secret, missing, locked (the lookups are issued in
    // the order the pod's containers name them: the key refs first, then `envFrom`).
    let _ = &port;
    let sources = svc
        .env_sources(&id("a"), &pod)
        .now_or_never()
        .unwrap()
        .unwrap();

    let calls: Vec<_> = port
        .recorded_calls()
        .into_iter()
        .map(|c| match c {
            ResourceCall::GetOpt { kind, name, .. } => (kind.kind.to_string(), name),
            other => panic!("only reads: {other:?}"),
        })
        .collect();
    assert_eq!(calls.len(), 4, "one get per object, not per key: {calls:?}");
    assert!(port.mutating_calls().is_empty());

    let secret = sources
        .get(SourceKind::Secret, &Arc::from("web-secret"))
        .unwrap();
    let SourceState::Found(data) = secret else {
        panic!("{secret:?}");
    };
    assert!(matches!(data.get("token"), Some(SourceValue::Secret(_))));
    assert_eq!(
        sources.get(SourceKind::ConfigMap, &Arc::from("missing")),
        Some(&SourceState::NotFound)
    );

    // Resolved through the view-model: the ConfigMap key shows, the Secret is masked, the
    // missing object has a chip.
    let rows = PodDetail::from_resource(&pod).unwrap().containers[0].env_rows(&pod, &sources);
    let row = |name: &str| rows.iter().find(|r| r.name == name).unwrap();
    assert_eq!(row("mode").value, EnvValue::Plain("prod".into()));
    assert!(row("TOKEN").is_secret());
    assert!(matches!(
        row("OTHER").value,
        EnvValue::Missing(Missing::Key { .. })
    ));
    assert!(matches!(
        row("GONE").value,
        EnvValue::Missing(Missing::Object { .. })
    ));
    // A secret's debug output never has its value.
    assert!(!format!("{sources:?}").contains("hunter2"));
}

#[test]
fn an_object_the_user_may_not_read_is_unavailable_not_an_error() {
    let t = Workloads::new(false);
    t.port().script().get_opt.push_err(OxiError::forbidden(
        "secrets \"locked\" is forbidden: User cannot get",
    ));
    let pod = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-0", "namespace": "default"},
        "spec": {"containers": [{"name": "app", "env": [
            {"name": "DENIED", "valueFrom": {"secretKeyRef": {"name": "locked", "key": "k"}}}]}]},
    }))
    .unwrap();
    let sources = service(&t)
        .env_sources(&id("a"), &pod)
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(
        sources.get(SourceKind::Secret, &Arc::from("locked")),
        Some(&SourceState::Unavailable("you may not read it".into()))
    );
}

#[test]
fn a_pod_without_references_looks_nothing_up() {
    let t = Workloads::new(false);
    let sources = service(&t)
        .env_sources(&id("a"), &pod("web-0", "Running"))
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(sources, Default::default());
    assert!(t.port().recorded_calls().is_empty());
}
