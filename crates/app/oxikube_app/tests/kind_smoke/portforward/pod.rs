//! Scenarios A and B on a pod target: GET nginx through the forward, then delete the pod; its
//! StatefulSet recreates it under the same name, and the forward goes `Reconnecting` and back to
//! `Active` on the same local port.

use std::time::{Duration, Instant};

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::DeleteParams;
use oxikube_domain::notification::ForwardPhase;
use oxikube_domain::portforward::ForwardPort;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::nginx;

use super::{Forwarding, active_pod, assert_reconnect, create, is_active, time_recovery};
use crate::cluster::Kind;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pod_forward_answers_and_recovers_when_its_pod_is_recreated() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let client = kind.admin_client().await;
    create::<StatefulSet>(&client, ns.name(), nginx::stateful_set("web", "web")).await;
    let pod = super::wait_ready_pod(&client, ns.name(), "web").await;
    assert_eq!(pod.name, "web-0");

    let forwarding = Forwarding::connect(&kind).await;
    let spec = forwarding.spec(
        "Pod",
        ns.name(),
        &pod.name,
        ForwardPort::Named(nginx::PORT_NAME.into()),
    );
    let info = forwarding
        .manager
        .start_and_wait(spec)
        .await
        .expect("the forward starts");
    let id = info.id;
    let info = forwarding.wait(id, "active", is_active).await;
    assert_eq!(active_pod(&info), Some("web-0"));
    let addr = info.local_addr.expect("bound");
    assert!(addr.ip().is_loopback(), "127.0.0.1 by default: {addr}");
    assert_ne!(addr.port(), 0, "Auto picked a real port");
    nginx::wait_welcome(addr, crate::DEADLINE).await;
    let served = forwarding.manager.get(id).expect("listed").stats;
    assert!(
        served.total_connections >= 1,
        "the connection is counted: {served:?}"
    );
    assert!(served.bytes_in > 0, "the page's bytes are counted");

    // B: the pod goes; the StatefulSet brings back `web-0`, a new pod under the same name.
    let deleted = Instant::now();
    Api::<Pod>::namespaced(client.clone(), ns.name())
        .delete("web-0", &DeleteParams::default())
        .await
        .expect("delete the pod");
    let recovery = time_recovery(&forwarding, &client, ns.name(), "web", id, deleted, &pod).await;
    assert_eq!(recovery.pod, "web-0");
    let info = forwarding.manager.get(id).expect("listed");
    assert_eq!(
        info.local_addr,
        Some(addr),
        "the local port is kept across the reconnect"
    );
    nginx::wait_welcome(addr, Duration::from_secs(10)).await;
    assert_reconnect("portforward.pod_recreated", &recovery);
    let transitions = forwarding.transitions(id);
    assert!(
        transitions.contains(&(ForwardPhase::Active, ForwardPhase::Reconnecting)),
        "the drop was reported: {transitions:?}"
    );
    assert!(
        transitions.contains(&(ForwardPhase::Reconnecting, ForwardPhase::Active)),
        "the recovery was reported: {transitions:?}"
    );
    assert!(forwarding.manager.remove(id));
}
