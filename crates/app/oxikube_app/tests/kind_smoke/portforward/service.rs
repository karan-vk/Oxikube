//! Scenario C (and B on a Service): a forward to a ClusterIP Service goes to the pod its
//! EndpointSlice lists as ready; deleting that pod moves it to the Deployment's replacement on the
//! same local port; scaling to zero leaves it waiting and scaling back brings it back.

use std::time::{Duration, Instant};

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams};
use kube::{Api, Client};
use oxikube_app::portforward::ForwardState;
use oxikube_domain::portforward::ForwardPort;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::nginx;
use serde_json::json;

use super::{Forwarding, active_pod, assert_reconnect, create, is_active, time_recovery};
use crate::cluster::Kind;

/// The pods the Service's EndpointSlices list as ready, by name.
async fn slice_ready_pods(client: &Client, namespace: &str, service: &str) -> Vec<String> {
    let params = ListParams::default().labels(&format!("kubernetes.io/service-name={service}"));
    let slices = Api::<EndpointSlice>::namespaced(client.clone(), namespace)
        .list(&params)
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    let mut pods: Vec<String> = slices
        .iter()
        .flat_map(|s| s.endpoints.iter().flatten())
        .filter(|e| e.conditions.as_ref().and_then(|c| c.ready) == Some(true))
        .filter_map(|e| e.target_ref.as_ref()?.name.clone())
        .collect();
    pods.sort();
    pods
}

async fn scale(client: &Client, namespace: &str, name: &str, replicas: u32) {
    Api::<Deployment>::namespaced(client.clone(), namespace)
        .patch_scale(
            name,
            &PatchParams::default(),
            &Patch::Merge(json!({"spec": {"replicas": replicas}})),
        )
        .await
        .expect("scale the deployment");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_service_forward_follows_ready_pods_through_replacement_and_scale_to_zero() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let client = kind.admin_client().await;
    create::<Deployment>(&client, ns.name(), nginx::deployment("web", "web", 1)).await;
    create::<Service>(&client, ns.name(), nginx::service("web", "web")).await;
    let first = super::wait_ready_pod(&client, ns.name(), "web").await;
    crate::eventually("the slice to list the ready pod", String::new, || async {
        slice_ready_pods(&client, ns.name(), "web").await == [first.name.clone()]
    })
    .await;

    let forwarding = Forwarding::connect(&kind).await;
    let spec = forwarding.spec(
        "Service",
        ns.name(),
        "web",
        ForwardPort::Number(nginx::SERVICE_PORT),
    );
    let id = forwarding
        .manager
        .start_and_wait(spec)
        .await
        .expect("the forward starts")
        .id;
    let info = forwarding.wait(id, "active", is_active).await;
    assert_eq!(
        active_pod(&info),
        Some(first.name.as_str()),
        "the ready pod of the EndpointSlice"
    );
    let addr = info.local_addr.expect("bound");
    nginx::wait_welcome(addr, crate::DEADLINE).await;

    // B: the Deployment replaces the deleted pod under a new name; the forward follows the slice.
    let deleted = Instant::now();
    Api::<Pod>::namespaced(client.clone(), ns.name())
        .delete(&first.name, &DeleteParams::default())
        .await
        .expect("delete the pod");
    let recovery = time_recovery(&forwarding, &client, ns.name(), "web", id, deleted, &first).await;
    assert!(
        slice_ready_pods(&client, ns.name(), "web")
            .await
            .contains(&recovery.pod),
        "the replacement {} is a ready endpoint",
        recovery.pod
    );
    assert_eq!(forwarding.manager.get(id).unwrap().local_addr, Some(addr));
    nginx::wait_welcome(addr, Duration::from_secs(10)).await;
    assert_reconnect("portforward.service_pod_replaced", &recovery);

    // Scale to zero: no ready pod, the forward waits (`Reconnecting`, not failed) on its port.
    scale(&client, ns.name(), "web", 0).await;
    let waiting = forwarding
        .wait(id, "waiting for a ready pod", |i| {
            matches!(i.state, ForwardState::Reconnecting { .. })
        })
        .await;
    assert_eq!(waiting.local_addr, Some(addr));
    crate::eventually("the old pod to go", String::new, || async {
        super::ready_pods(&client, ns.name(), "web")
            .await
            .is_empty()
    })
    .await;
    assert!(
        !matches!(
            forwarding.manager.get(id).unwrap().state,
            ForwardState::Failed(_) | ForwardState::Active { .. }
        ),
        "still waiting with no pod"
    );

    // And back: the new pod serves on the same port.
    let scaled = Instant::now();
    scale(&client, ns.name(), "web", 1).await;
    let gone = super::PodId {
        name: recovery.pod.clone(),
        uid: String::new(),
    };
    let back = time_recovery(&forwarding, &client, ns.name(), "web", id, scaled, &gone).await;
    nginx::wait_welcome(addr, Duration::from_secs(10)).await;
    assert_reconnect("portforward.service_scaled_back", &back);
    assert!(forwarding.manager.remove(id));
}
