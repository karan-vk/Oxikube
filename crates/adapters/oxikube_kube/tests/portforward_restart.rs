//! Kind integration for E04-S10: the restart hook. A deleted pod is `TargetGone`; a service
//! forward recovers on the replacement pod, a pod forward stops.
//! Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::DeleteParams;
use oxikube_domain::{ForwardPort, ForwardStatus};
use oxikube_kube::KubePortForward;
use oxikube_testkit::integration::TestNamespace;
use std::time::Duration;
use tokio::sync::broadcast::Receiver;

use common::portforward::{
    create_nginx_deployment, create_nginx_pod, create_nginx_service, get_welcome_page, live_pods,
    spec, wait_ready,
};
use common::wait_until;

/// Pod deletion, replacement scheduling and image start on a busy kind node.
const RECOVERY: Duration = Duration::from_secs(120);

/// The next status event satisfying `pred`, skipping others (`Error` from a connection in
/// flight when the pod died is allowed to come first).
async fn next_matching(
    events: &mut Receiver<ForwardStatus>,
    pred: impl Fn(&ForwardStatus) -> bool,
) -> ForwardStatus {
    tokio::time::timeout(RECOVERY, async {
        loop {
            let status = events.recv().await.expect("the forward is publishing");
            if pred(&status) {
                return status;
            }
        }
    })
    .await
    .expect("the expected status arrived in time")
}

#[tokio::test]
async fn deleting_the_pod_behind_a_service_forward_reports_target_gone_then_recovers() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_deployment(&client, ns.name(), "web", "web").await;
    create_nginx_service(&client, ns.name(), "web", "web").await;
    let first = wait_until("the first pod exists", RECOVERY, || async {
        live_pods(&client, ns.name(), "web").await.pop()
    })
    .await;
    wait_ready(&client, ns.name(), &first).await;

    let handle = KubePortForward::new((*client).clone())
        .start(&spec("Service", ns.name(), "web", ForwardPort::Number(80)))
        .await
        .expect("start");
    let mut events = handle.events();
    get_welcome_page(handle.local_addr()).await;

    // Delete it the way a rollout or an eviction would; the ReplicaSet makes another.
    Api::<Pod>::namespaced((*client).clone(), ns.name())
        .delete(&first, &DeleteParams::default().grace_period(0))
        .await
        .expect("delete the pod");

    let gone = next_matching(&mut events, |s| {
        matches!(s, ForwardStatus::TargetGone { .. })
    })
    .await;
    assert_eq!(gone, ForwardStatus::TargetGone { pod: first.clone() });
    let recovered = next_matching(&mut events, |s| {
        matches!(s, ForwardStatus::Listening { .. })
    })
    .await;
    let ForwardStatus::Listening { pod, .. } = recovered else {
        unreachable!()
    };
    assert_ne!(pod, first, "moved to the replacement pod");

    // Same local port, new pod.
    let response = get_welcome_page(handle.local_addr()).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(!handle.is_finished());
}

#[tokio::test]
async fn deleting_the_pod_of_a_pod_forward_reports_target_gone_then_stops() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_nginx_pod(&client, ns.name(), "web", "web").await;
    wait_ready(&client, ns.name(), "web").await;

    let handle = KubePortForward::new((*client).clone())
        .start(&spec("Pod", ns.name(), "web", ForwardPort::Number(80)))
        .await
        .expect("start");
    let mut events = handle.events();
    let addr = handle.local_addr();
    get_welcome_page(addr).await;

    Api::<Pod>::namespaced((*client).clone(), ns.name())
        .delete("web", &DeleteParams::default().grace_period(0))
        .await
        .expect("delete the pod");

    next_matching(&mut events, |s| {
        matches!(s, ForwardStatus::TargetGone { .. })
    })
    .await;
    next_matching(&mut events, ForwardStatus::is_terminal).await;
    wait_until("the listener is closed", RECOVERY, || async {
        (handle.is_finished() && tokio::net::TcpStream::connect(addr).await.is_err()).then_some(())
    })
    .await;
}
