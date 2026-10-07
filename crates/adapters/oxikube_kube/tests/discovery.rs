//! Kind integration tests for `KubeDiscovery` (E03-S06), part of the E03-S09 suite: the
//! discovery snapshot (a stable subset plus the fixture CRD) and a CRD added at runtime.
//! Clients come from the `ClientPool`. Need `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skip cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::time::{Duration, Instant};

use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_kube::{CrdWatchConfig, DiscoveryConfig, KubeDiscovery, RegistryDiff};
use oxikube_ports::{DiscoveryEvent, DiscoveryPort};
use tokio::sync::broadcast::Receiver;
use tokio::time::timeout;

use common::{DEADLINE, TestCrd};

/// Waits for a published diff for which `wanted` holds, within [`DEADLINE`].
async fn next_diff(
    changes: &mut Receiver<std::sync::Arc<RegistryDiff>>,
    what: &str,
    wanted: impl Fn(&RegistryDiff) -> bool,
) -> std::sync::Arc<RegistryDiff> {
    let started = Instant::now();
    loop {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        match timeout(remaining, changes.recv()).await {
            Ok(Ok(diff)) if wanted(&diff) => return diff,
            Ok(Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(err)) => panic!("change channel closed while waiting for {what}: {err}"),
            Err(_) => panic!("{what} not seen within {DEADLINE:?}"),
        }
    }
}

#[tokio::test]
async fn registry_lists_builtin_kinds_and_the_fixture_crd() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let discovery = KubeDiscovery::new((*kind.admin_client().await).clone());

    let started = Instant::now();
    let kinds = discovery.discover().await.expect("discover");
    let elapsed = started.elapsed();
    eprintln!("aggregated discovery: {} kinds in {elapsed:?}", kinds.len());

    let get = |group: &str, version: &str, kind: &str| {
        kinds
            .iter()
            .find(|k| k.gvk == Gvk::new(group, version, kind))
            .unwrap_or_else(|| panic!("{group}/{version} {kind} not discovered"))
    };
    let pod = get("", "v1", "Pod");
    assert!(pod.namespaced && pod.preferred);
    assert_eq!(pod.short_names, ["po"]);
    assert!(pod.categories.iter().any(|c| c == "all"));
    assert!(pod.supports(Verb::List) && pod.is_watchable());

    let deployment = get("apps", "v1", "Deployment");
    assert_eq!(deployment.short_names, ["deploy"]);
    assert_eq!(deployment.plural, "deployments");

    assert!(!get("", "v1", "Namespace").namespaced);

    let widget = get("test.oxikube.dev", "v1", "Widget");
    assert_eq!(widget.plural, "widgets");
    assert!(widget.namespaced && widget.preferred);

    assert!(
        kinds.iter().all(|k| !k.plural.contains('/')),
        "subresources are not listed"
    );
}

#[tokio::test]
async fn aggregated_and_legacy_discovery_agree_on_a_real_cluster() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = (*kind.admin_client().await).clone();
    let aggregated = KubeDiscovery::new(client.clone());
    let legacy = KubeDiscovery::with_config(
        client,
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );

    let started = Instant::now();
    let from_aggregated = aggregated.discover().await.expect("aggregated");
    let aggregated_time = started.elapsed();
    let started = Instant::now();
    let from_legacy = legacy.discover().await.expect("legacy");
    let legacy_time = started.elapsed();
    eprintln!(
        "discovery on kind: aggregated {aggregated_time:?}, legacy fallback {legacy_time:?} ({} kinds)",
        from_aggregated.len()
    );

    // Sibling tests create and delete `rt-*` CRDs concurrently; compare only what is stable.
    let stable = |kinds: Vec<oxikube_domain::kinds::ResourceKind>| {
        kinds
            .into_iter()
            .filter(|k| !k.gvk.group.starts_with("rt-"))
            .collect::<Vec<_>>()
    };
    assert_eq!(stable(from_aggregated), stable(from_legacy));
}

#[tokio::test]
async fn a_crd_created_at_runtime_appears_and_removal_is_reported() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ctx = kind.context.as_str();
    let client = (*kind.admin_client().await).clone();
    let discovery = KubeDiscovery::new(client.clone());
    discovery.discover().await.expect("discover");
    let mut changes = discovery.registry_changes();
    let _watch = discovery.watch_crds(CrdWatchConfig {
        debounce: Duration::from_millis(100),
        ..CrdWatchConfig::default()
    });

    let crd = TestCrd::create(&client, ctx).await;
    let diff = next_diff(&mut changes, "the new CRD's kind", |d| {
        d.added.iter().any(|k| k.gvk == crd.gvk)
    })
    .await;
    let gizmo = diff.added.iter().find(|k| k.gvk == crd.gvk).expect("added");
    assert_eq!(
        (gizmo.plural.as_str(), gizmo.short_names.as_slice()),
        ("gizmos", ["gz".to_owned()].as_slice())
    );
    assert!(discovery.registry().get(&crd.gvk).is_some());
    assert!(
        discovery
            .resolve(&crd.gvk)
            .await
            .expect("resolve")
            .is_some()
    );

    crd.delete(&client).await;
    let diff = next_diff(&mut changes, "removal of the CRD's kind", |d| {
        d.removed.iter().any(|k| k.gvk == crd.gvk)
    })
    .await;
    assert!(diff.added.iter().all(|k| k.gvk != crd.gvk));
    assert!(discovery.registry().get(&crd.gvk).is_none());
}

#[tokio::test]
async fn a_resolve_miss_finds_a_crd_created_after_discovery() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ctx = kind.context.as_str();
    let client = (*kind.admin_client().await).clone();
    let discovery = KubeDiscovery::with_config(
        client.clone(),
        DiscoveryConfig {
            miss_cooldown: Duration::ZERO,
            ..DiscoveryConfig::default()
        },
    );
    discovery.discover().await.expect("discover");

    let crd = TestCrd::create(&client, ctx).await;
    // A CRD is served a moment after creation (until it is established, discovery omits it),
    // so poll `resolve` until the deadline.
    let started = Instant::now();
    loop {
        if discovery
            .resolve(&crd.gvk)
            .await
            .expect("resolve")
            .is_some()
        {
            break;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "CRD not served within {DEADLINE:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    crd.delete(&client).await;
}

/// The app's path (E03-F544): the session subscribes through the port and the adapter starts the
/// CRD watch itself; a CRD created and deleted at runtime is reported and resolvable.
#[tokio::test]
async fn subscribing_through_the_port_follows_a_crd_added_and_removed() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ctx = kind.context.as_str();
    let client = (*kind.admin_client().await).clone();
    let discovery = KubeDiscovery::new(client.clone());
    discovery.discover().await.expect("discover");
    let mut events = discovery.subscribe();

    let crd = TestCrd::create(&client, ctx).await;
    next_event(
        &mut events,
        "the new CRD's kind",
        |e| matches!(e, DiscoveryEvent::KindsChanged(c) if c.added.contains(&crd.gvk)),
    )
    .await;
    assert!(
        discovery
            .resolve(&crd.gvk)
            .await
            .expect("resolve")
            .is_some(),
        "the registry knows the kind once the event arrived"
    );

    crd.delete(&client).await;
    next_event(
        &mut events,
        "removal of the CRD's kind",
        |e| matches!(e, DiscoveryEvent::KindsChanged(c) if c.removed.contains(&crd.gvk)),
    )
    .await;
    assert!(discovery.registry().get(&crd.gvk).is_none());

    // Dropping the stream stops the watch: the status channel keeps its last value and no task
    // lingers to publish (nothing to assert beyond not hanging the runtime on drop).
    drop(events);
}

async fn next_event(
    events: &mut oxikube_ports::DiscoveryEvents,
    what: &str,
    wanted: impl Fn(&DiscoveryEvent) -> bool,
) {
    use futures::StreamExt as _;
    let started = Instant::now();
    loop {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        match timeout(remaining, events.next()).await {
            Ok(Some(event)) if wanted(&event) => return,
            Ok(Some(_)) => {}
            Ok(None) => panic!("event stream ended while waiting for {what}"),
            Err(_) => panic!("{what} not seen within {DEADLINE:?}"),
        }
    }
}
