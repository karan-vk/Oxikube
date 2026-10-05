//! Fixtures for the port-forward scenarios (E04-S10): nginx pods, services and deployments,
//! and an HTTP GET over a local forward.

use std::net::SocketAddr;
use std::time::Duration;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Pod, Service};
use kube::api::PostParams;
use kube::{Api, Client};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{ForwardPort, ForwardSpec};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{DEADLINE, wait_until};

/// Small, and serves its welcome page on port 80 as soon as it starts.
pub const NGINX_IMAGE: &str = "nginx:1.27-alpine";

fn nginx_container() -> Value {
    json!({
        "name": "nginx", "image": NGINX_IMAGE,
        "ports": [{"name": "http", "containerPort": 80}],
        // Ready only once nginx answers, so "Ready" means "the forward will work".
        "readinessProbe": {"httpGet": {"path": "/", "port": 80}, "periodSeconds": 1},
    })
}

/// A pod `name` labelled `app=<app>` running nginx.
pub async fn create_nginx_pod(client: &Client, namespace: &str, name: &str, app: &str) {
    let pod: Pod = serde_json::from_value(json!({
        "metadata": {"name": name, "labels": {"app": app}},
        "spec": {"terminationGracePeriodSeconds": 1, "containers": [nginx_container()]},
    }))
    .expect("pod");
    Api::<Pod>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &pod)
        .await
        .expect("create pod");
}

/// A service `name` selecting `app=<app>`: port 80 to the named container port `http`.
pub async fn create_nginx_service(client: &Client, namespace: &str, name: &str, app: &str) {
    let service: Service = serde_json::from_value(json!({
        "metadata": {"name": name},
        "spec": {"selector": {"app": app},
                 "ports": [{"name": "web", "port": 80, "targetPort": "http"}]},
    }))
    .expect("service");
    Api::<Service>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &service)
        .await
        .expect("create service");
}

/// A one-replica deployment `name` whose pods carry `app=<app>`.
pub async fn create_nginx_deployment(client: &Client, namespace: &str, name: &str, app: &str) {
    let deployment: Deployment = serde_json::from_value(json!({
        "metadata": {"name": name},
        "spec": {
            "replicas": 1,
            "selector": {"matchLabels": {"app": app}},
            "template": {
                "metadata": {"labels": {"app": app}},
                "spec": {"terminationGracePeriodSeconds": 1, "containers": [nginx_container()]},
            },
        },
    }))
    .expect("deployment");
    Api::<Deployment>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &deployment)
        .await
        .expect("create deployment");
}

/// Waits until the pod `name` is Ready.
pub async fn wait_ready(client: &Client, namespace: &str, name: &str) {
    let pods = Api::<Pod>::namespaced(client.clone(), namespace);
    wait_until(
        &format!("pod {name} ready"),
        Duration::from_secs(120),
        || async {
            let pod = pods.get_opt(name).await.ok()??;
            let ready = pod
                .status?
                .conditions?
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True");
            ready.then_some(())
        },
    )
    .await;
}

/// The names of the pods labelled `app=<app>` that are not terminating.
pub async fn live_pods(client: &Client, namespace: &str, app: &str) -> Vec<String> {
    let params = kube::api::ListParams::default().labels(&format!("app={app}"));
    Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&params)
        .await
        .map(|list| {
            list.items
                .into_iter()
                .filter(|p| p.metadata.deletion_timestamp.is_none())
                .filter_map(|p| p.metadata.name)
                .collect()
        })
        .unwrap_or_default()
}

/// A forward spec for the pod or service `name` in `namespace`.
pub fn spec(kind: &str, namespace: &str, name: &str, remote: ForwardPort) -> ForwardSpec {
    ForwardSpec::new(
        ResourceRef::new(
            ClusterId::new("kind-test", &ContextName::from("kind-oxikube")),
            Gvk::new("", "v1", kind),
            Some(namespace.into()),
            name,
        ),
        remote,
    )
}

/// One `GET /` through `addr`, the whole response.
pub async fn http_get(addr: SocketAddr) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(addr).await?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut response = String::new();
    tokio::time::timeout(
        Duration::from_secs(10),
        stream.read_to_string(&mut response),
    )
    .await
    .map_err(|_| std::io::ErrorKind::TimedOut)??;
    Ok(response)
}

/// Retries [`http_get`] until nginx's welcome page comes back through the forward.
pub async fn get_welcome_page(addr: SocketAddr) -> String {
    wait_until("nginx answers through the forward", DEADLINE, || async {
        http_get(addr)
            .await
            .ok()
            .filter(|r| r.contains("Welcome to nginx!"))
    })
    .await
}
