//! nginx fixtures and HTTP assertions for the port-forward kind suites (E15-S09), as JSON ready
//! for `serde_json::from_value::<Deployment>` (or `Pod`, `StatefulSet`, `Service`) or
//! `kubectl create -f -`.
//!
//! Every pod runs nginx ([`images::NGINX`], pulled by `cargo xtask kind-up`) with one container
//! port named [`PORT_NAME`] (80) and a readiness probe on `/`, so "Ready" means "a forward to it
//! answers". Pods are labelled `app=<app>` for the Service selector and [`SUITE_LABEL`]`=nginx`
//! so a leftover is easy to find. No termination grace period, so a delete is quick.
//!
//! | Fixture | What it is for |
//! |---|---|
//! | [`pod`] | a bare pod: forward to a pod, error cases |
//! | [`deployment`] | pods with new names after a delete: a Service forward re-resolving |
//! | [`stateful_set`] | a pod that comes back with the same name (`<name>-0`): a pod forward recovering |
//! | [`service`] | a ClusterIP Service: port [`SERVICE_PORT`] to the named container port |
//!
//! [`http_get`] and [`wait_welcome`] talk HTTP/1.1 over a plain TCP connection to the forward's
//! local address; no HTTP client crate, nothing logged but the response's status line.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use super::pods::SUITE_LABEL;
use crate::images;

/// The container port nginx serves on.
pub const CONTAINER_PORT: u16 = 80;

/// The name of [`CONTAINER_PORT`] in every fixture.
pub const PORT_NAME: &str = "http";

/// The port of [`service`] (its `targetPort` is [`PORT_NAME`]).
pub const SERVICE_PORT: u16 = 8080;

/// What nginx's welcome page says.
pub const WELCOME: &str = "Welcome to nginx!";

fn pod_spec() -> Value {
    json!({
        "terminationGracePeriodSeconds": 0,
        "containers": [{
            "name": "nginx",
            "image": images::NGINX,
            "ports": [{"name": PORT_NAME, "containerPort": CONTAINER_PORT}],
            "readinessProbe": {
                "httpGet": {"path": "/", "port": CONTAINER_PORT},
                "periodSeconds": 1,
            },
        }],
    })
}

fn labels(app: &str) -> Value {
    json!({ "app": app, SUITE_LABEL: "nginx" })
}

/// A bare nginx pod `name` labelled `app=<app>`.
pub fn pod(name: &str, app: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": name, "labels": labels(app)},
        "spec": pod_spec(),
    })
}

/// A Deployment `name` of `replicas` nginx pods labelled `app=<app>`.
pub fn deployment(name: &str, app: &str, replicas: u32) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": name, "labels": labels(app)},
        "spec": {
            "replicas": replicas,
            "selector": {"matchLabels": {"app": app}},
            "template": {"metadata": {"labels": labels(app)}, "spec": pod_spec()},
        },
    })
}

/// A one-replica StatefulSet `name` (its pod is `<name>-0`, recreated with that name when
/// deleted) labelled `app=<app>`.
pub fn stateful_set(name: &str, app: &str) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "StatefulSet",
        "metadata": {"name": name, "labels": labels(app)},
        "spec": {
            "replicas": 1,
            "serviceName": name,
            "podManagementPolicy": "Parallel",
            "selector": {"matchLabels": {"app": app}},
            "template": {"metadata": {"labels": labels(app)}, "spec": pod_spec()},
        },
    })
}

/// A ClusterIP Service `name` selecting `app=<app>`: port [`SERVICE_PORT`] (named `web`) to the
/// named container port [`PORT_NAME`].
pub fn service(name: &str, app: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {"name": name, "labels": {SUITE_LABEL: "nginx"}},
        "spec": {
            "selector": {"app": app},
            "ports": [{"name": "web", "port": SERVICE_PORT, "targetPort": PORT_NAME}],
        },
    })
}

/// One `GET /` through `addr`, the whole response (headers and body), within `timeout`.
///
/// # Errors
///
/// The connection's error, or `TimedOut`.
pub async fn http_get(addr: SocketAddr, timeout: Duration) -> std::io::Result<String> {
    let exchange = async {
        let mut stream = TcpStream::connect(addr).await?;
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;
        Ok(response)
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))?
}

/// Whether `response` is nginx's welcome page with status 200.
pub fn is_welcome(response: &str) -> bool {
    response.starts_with("HTTP/1.1 200") && response.contains(WELCOME)
}

/// Retries [`http_get`] through `addr` every 100 ms until nginx's welcome page comes back, and
/// returns how long that took.
///
/// # Panics
///
/// After `deadline` without the page, naming the last error or status line.
pub async fn wait_welcome(addr: SocketAddr, deadline: Duration) -> Duration {
    let started = Instant::now();
    loop {
        let last = match http_get(addr, Duration::from_secs(5)).await {
            Ok(response) if is_welcome(&response) => return started.elapsed(),
            Ok(response) => response.lines().next().unwrap_or_default().to_owned(),
            Err(error) => error.to_string(),
        };
        assert!(
            started.elapsed() < deadline,
            "no nginx welcome page through {addr} after {deadline:?}; last: {last}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixtures_name_their_port_and_image() {
        for manifest in [
            pod("web", "web"),
            deployment("web", "web", 1)["spec"]["template"].clone(),
            stateful_set("web", "web")["spec"]["template"].clone(),
        ] {
            let container = &manifest["spec"]["containers"][0];
            assert_eq!(container["image"], images::NGINX);
            assert_eq!(container["ports"][0]["name"], PORT_NAME);
            assert_eq!(manifest["metadata"]["labels"]["app"], "web");
        }
        let svc = service("web", "web");
        assert_eq!(svc["spec"]["ports"][0]["targetPort"], PORT_NAME);
        assert_eq!(svc["spec"]["selector"]["app"], "web");
    }

    #[test]
    fn a_welcome_page_needs_a_200() {
        assert!(is_welcome(
            "HTTP/1.1 200 OK\r\n\r\n<h1>Welcome to nginx!</h1>"
        ));
        assert!(!is_welcome(
            "HTTP/1.1 502 Bad Gateway\r\n\r\nWelcome to nginx!"
        ));
        assert!(!is_welcome("HTTP/1.1 200 OK\r\n\r\nhello"));
    }

    #[tokio::test]
    async fn wait_welcome_retries_until_the_page_comes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for answer in [
                "HTTP/1.1 503 Busy\r\n\r\n",
                "HTTP/1.1 200 OK\r\n\r\nWelcome to nginx!",
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 256];
                let _ = socket.read(&mut buf).await.unwrap();
                socket.write_all(answer.as_bytes()).await.unwrap();
            }
        });
        let took = wait_welcome(addr, Duration::from_secs(5)).await;
        assert!(took < Duration::from_secs(5));
    }
}
