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
    assert!(
        started.url().starts_with("http://127.0.0.1:"),
        "{}",
        started.url()
    );
    let info = rig.manager.get(started.id).expect("listed");
    assert_eq!(info.spec.name.as_deref(), Some("my api"));
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
