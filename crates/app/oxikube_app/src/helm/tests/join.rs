//! Joining with the `ResourceStore`: health, `Missing`, `Unknown`, live updates, lease lifetime.

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::helm::parse_manifest;
use oxikube_domain::view::Health;
use oxikube_ports::Delta;
use oxikube_testkit::{Timeline, deployment, pod};

use super::{Harness, object, release, served};
use crate::helm::{ReleaseResourcesLease, ResourcePlan, ResourceState, UnknownReason};

const MANIFEST: &str = r#"---
apiVersion: v1
kind: ConfigMap
metadata:
  name: cfg
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
---
apiVersion: v1
kind: Pod
metadata:
  name: standalone
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: gone
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: web-role
---
apiVersion: example.com/v1
kind: Widget
metadata:
  name: w
"#;

fn plan() -> Arc<ResourcePlan> {
    Arc::new(ResourcePlan::build(
        &release(),
        &parse_manifest(MANIFEST),
        &served(),
    ))
}

#[tokio::test(flavor = "current_thread")]
async fn present_missing_and_unknown_are_told_apart() {
    let h = Harness::new();
    h.resources
        .insert(object("v1", "ConfigMap", Some("apps"), "cfg"));
    h.resources
        .insert(deployment().namespace("apps").name("web").ready(0).build());
    h.resources
        .insert(pod().namespace("apps").name("standalone").running().build());
    h.resources
        .insert(object("v1", "ServiceAccount", Some("apps"), "other"));
    h.resources.insert(object(
        "rbac.authorization.k8s.io/v1",
        "ClusterRole",
        None,
        "web-role",
    ));

    let lease = ReleaseResourcesLease::new(&h.store, plan());
    // Before any feed has listed nothing is claimed missing.
    assert!(
        lease
            .resources()
            .iter()
            .all(|r| matches!(r.state, ResourceState::Unknown(_)))
    );
    h.settle().await;
    let rows = lease.resources();
    let by_name = |n: &str| rows.iter().find(|r| &*r.reference.name == n).unwrap();

    // A kind without a health rule is present with a neutral status.
    assert!(matches!(
        by_name("cfg").state,
        ResourceState::Present { health: None, .. }
    ));
    // Health comes from the shared view-model rules.
    assert!(matches!(
        by_name("web").state,
        ResourceState::Present {
            health: Some(Health::Unhealthy),
            ..
        }
    ));
    assert!(matches!(
        by_name("standalone").state,
        ResourceState::Present {
            health: Some(Health::Healthy),
            ..
        }
    ));
    assert!(matches!(
        by_name("web-role").state,
        ResourceState::Present { .. }
    ));
    // The manifest lists it, the cluster does not have it.
    assert!(by_name("gone").state.is_missing());
    // A CRD that is not installed.
    assert!(matches!(
        by_name("w").state,
        ResourceState::Unknown(UnknownReason::KindNotServed)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn store_updates_change_the_result() {
    let h = Harness::new();
    let one = |ready: bool| {
        let mut p = pod().namespace("apps").name("standalone");
        p = if ready { p.running() } else { p.pending() };
        let mut r = p.build();
        r.meta.resource_version = Some(if ready { "2" } else { "1" }.into());
        r
    };
    h.resources.script().watch.push_ok(
        Timeline::new()
            .ok_at(
                Duration::ZERO,
                oxikube_ports::DeltaBatch::from_deltas(vec![Delta::Restarted(vec![one(false)])]),
            )
            .ok_at(
                Duration::from_secs(1),
                oxikube_ports::DeltaBatch::from_deltas(vec![Delta::Applied(one(true))]),
            )
            .ok_at(
                Duration::from_secs(2),
                oxikube_ports::DeltaBatch::from_deltas(vec![Delta::Deleted(one(true))]),
            )
            .keep_open(),
    );
    let plan = Arc::new(ResourcePlan::build(
        &release(),
        &parse_manifest("apiVersion: v1\nkind: Pod\nmetadata:\n  name: standalone\n"),
        &served(),
    ));
    let lease = ReleaseResourcesLease::new(&h.store, plan);
    h.settle().await;
    let state = |lease: &ReleaseResourcesLease| lease.resources().remove(0).state;
    assert!(matches!(
        state(&lease),
        ResourceState::Present {
            health: Some(Health::Unhealthy),
            ..
        }
    ));
    h.clock.advance(Duration::from_secs(1));
    h.settle().await;
    assert!(matches!(
        state(&lease),
        ResourceState::Present {
            health: Some(Health::Healthy),
            ..
        }
    ));
    h.clock.advance(Duration::from_secs(1));
    h.settle().await;
    assert!(state(&lease).is_missing());
}

#[tokio::test(flavor = "current_thread")]
async fn the_lease_holds_namespace_scoped_feeds_and_releases_them_on_drop() {
    let h = Harness::new();
    h.resources
        .insert(object("v1", "ConfigMap", Some("apps"), "cfg"));
    let lease = ReleaseResourcesLease::new(&h.store, plan());
    h.settle().await;
    let feeds = h.store.feeds();
    // ConfigMap, Deployment, Pod, ServiceAccount (per namespace) and ClusterRole (cluster-wide);
    // the CRD that is not served opens nothing.
    assert_eq!(feeds.len(), 5, "{feeds:?}");
    assert!(h.resources.live_watches() > 0);
    drop(lease);
    h.clock.advance(Duration::from_secs(600));
    h.settle().await;
    assert_eq!(h.resources.live_watches(), 0);
}
