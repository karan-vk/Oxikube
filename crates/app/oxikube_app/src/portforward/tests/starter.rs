//! `ForwardStarter`: the dialog's start through the bus, and the typed error it hands back.

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{Rig, pod, pod_spec, target};
use crate::command_bus::{CommandBus, CommandRegistry};
use crate::guard::MutationGuard;
use crate::portforward::{ForwardStarter, StartError, register_commands};
use crate::session::ClusterSessionManager;

fn starter(rig: &Rig) -> ForwardStarter {
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, rig.manager.clone()).expect("registered");
    ForwardStarter::new(
        CommandBus::new(registry, guard),
        "alice",
        rig.manager.clone(),
    )
}

#[tokio::test]
async fn a_start_goes_through_portforward_start_and_carries_name_and_favourite() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let started = starter(&rig)
        .start(
            pod_spec("api-0", 80)
                .with_name("my api")
                .with_favourite(true),
        )
        .await
        .expect("started");
    let url = started.url().expect("bound");
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    let info = rig.manager.get(started.id).expect("listed");
    assert_eq!(info.spec.name.as_deref(), Some("my api"));
    assert_eq!(info.label(), "my api", "notifications use the user's name");
    assert!(info.spec.favourite);
    assert_eq!(started.pod.as_deref(), Some("api-0"));
}

#[tokio::test]
async fn a_read_only_cluster_is_blocked_up_front_and_refused_with_a_typed_error() {
    let rig = Rig::new();
    *rig.backend.read_only.lock() = true;
    let starter = starter(&rig);
    let reason = starter
        .blocked(&target("Pod", "api-0").cluster)
        .expect("blocked");
    assert!(reason.contains("allowed_in_readonly"), "{reason}");
    let error = starter.start(pod_spec("api-0", 80)).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(matches!(
        StartError::of(&error),
        Some(StartError::ReadOnly(_))
    ));

    rig.manager.set_config(crate::portforward::ForwardConfig {
        allowed_in_readonly: true,
        ..Default::default()
    });
    assert!(starter.blocked(&target("Pod", "api-0").cluster).is_none());
}

#[tokio::test]
async fn a_failed_start_leaves_nothing_listed_so_a_retry_does_not_duplicate() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port();
    let starter = starter(&rig);
    let error = starter
        .start(pod_spec("api-0", 80).with_local_port(port))
        .await
        .unwrap_err();
    assert!(StartError::of(&error).is_some());
    assert!(rig.manager.list().is_empty());
    starter
        .start(pod_spec("api-0", 80))
        .await
        .expect("the retry on an automatic port");
    assert_eq!(rig.manager.list().len(), 1);
}

#[tokio::test]
async fn a_forward_still_waiting_for_a_pod_is_a_pending_start_not_a_failure() {
    let rig = Rig::new();
    // The Pod feed never lists: the start's patience runs out with the forward still `Starting`.
    let starter = starter(&rig);
    let start = starter.start(pod_spec("api-0", 80));
    let advance = async {
        for _ in 0..200 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            rig.clock.advance(std::time::Duration::from_secs(1));
        }
    };
    let (started, ()) = futures::join!(start, advance);
    let started = started.expect("accepted");
    assert!(started.local_addr.is_none() && started.url().is_none());
    assert_eq!(
        rig.manager.list().len(),
        1,
        "it stays listed and keeps trying"
    );
}

#[tokio::test]
async fn annotation_suggestions_follow_the_setting_and_see_the_listed_forwards() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let starter = starter(&rig);
    let annotated = oxikube_testkit::resource("v1", "Pod")
        .name("api-0")
        .namespace(super::NS)
        .annotation("k9scli.io/auto-port-forwards", "app::18080:8080")
        .build();

    assert!(!starter.annotations(), "off by default");
    assert!(starter.annotation_suggestions(&annotated).is_empty());

    rig.manager.set_config(crate::portforward::ForwardConfig {
        annotations: true,
        ..Default::default()
    });
    let found = starter.annotation_suggestions(&annotated);
    assert_eq!(found.len(), 1);
    assert!(found[0].forward.auto && found[0].conflict.is_none());

    let live = starter
        .start(pod_spec("api-0", 8080).with_local_port(18080))
        .await
        .expect("started");
    assert!(live.local_addr.is_some());
    let found = starter.annotation_suggestions(&annotated);
    assert!(
        found[0]
            .conflict
            .as_deref()
            .is_some_and(|c| c.contains("18080")),
        "{found:?}"
    );
}

#[tokio::test]
async fn a_favourite_start_without_favourites_wired_still_forwards_and_says_why() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let started = starter(&rig)
        .start(pod_spec("api-0", 80).with_favourite(true))
        .await
        .expect("the forward runs regardless");
    assert!(started.local_addr.is_some());
    let why = started
        .favourite_error
        .expect("the save failure is reported");
    assert!(why.contains("favourites"), "{why}");
}

/// Audit E15 (#18): the dialog prefilled 127.0.0.1 whatever `portforward.default_bind` said.
#[tokio::test]
async fn the_default_bind_follows_the_setting_and_its_cluster_override() {
    let rig = Rig::new();
    let starter = starter(&rig);
    let cluster = target("Pod", "api-0").cluster;
    assert_eq!(starter.default_bind(&cluster).to_string(), "127.0.0.1");
    rig.manager.set_config(crate::portforward::ForwardConfig {
        default_bind: "0.0.0.0".parse().unwrap(),
        ..Default::default()
    });
    assert_eq!(starter.default_bind(&cluster).to_string(), "0.0.0.0");
    rig.manager
        .set_cluster_configs(std::collections::HashMap::from([(
            cluster.clone(),
            crate::portforward::ForwardConfig {
                default_bind: "::1".parse().unwrap(),
                ..Default::default()
            },
        )]));
    assert_eq!(starter.default_bind(&cluster).to_string(), "::1");
}
