//! The agent's log hooks (E08-S09) over the real adapters: `k8s.get_logs` through the
//! `ToolRegistry` and `@logs` through the `ContextRegistry`, reading busybox pods that echo
//! numbered lines (and one line with a bearer token) on the Tokio runtime and the wall clock.
//!
//! * a pod read returns the newest lines, honours `tail`, `since` and `grep`, and ends by itself
//!   (a tool call never follows, even though the pod keeps writing);
//! * the token in the pod's output is masked in everything handed to the agent;
//! * a label selector reads every pod it picks, each line attributed to its pod;
//! * a pod that does not exist is an error the model sees.
//!
//! Everything lives in the test's own `oxi-test-<rand>` namespace.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::PostParams;
use oxikube_app::context::{ContextRegistry, LogContextProvider};
use oxikube_app::logs::{
    AggregatePorts, LogCluster, LogClusters, LogConfig, LogRuntime, LogService,
};
use oxikube_app::store::Spawner;
use oxikube_app::tools::ToolRegistry;
use oxikube_app::tools::k8s::GetLogsTool;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_kube::{KubeDiscovery, KubeLogs, KubeResources};
use oxikube_ports::{ContextScope, ToolContext};
use oxikube_testkit::images::BUSYBOX;
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::eventually;

const SECRET: &str = "eyJhbGciOiJIUzI1NiJ9.e30.abcdefghijklmnopqrstuv";
const LABEL: &str = "oxikube.test/suite=agent-logs";

fn echo_pod(name: &str) -> Pod {
    let script = format!(
        "echo \"calling upstream Authorization: Bearer {SECRET}\"; \
         i=0; while true; do echo \"{name} line $i\"; i=$((i+1)); sleep 0.1; done"
    );
    serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { "oxikube.test/suite": "agent-logs" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{ "name": "main", "image": BUSYBOX, "command": ["sh", "-c", script] }],
        },
    }))
    .expect("a pod")
}

struct Fixed(LogCluster);

impl LogClusters for Fixed {
    fn cluster(&self, _: Option<&ClusterId>) -> OxiResult<LogCluster> {
        Ok(self.0.clone())
    }
}

async fn running(pods: &Api<Pod>, name: &str) -> bool {
    pods.get(name)
        .await
        .ok()
        .and_then(|p| p.status)
        .and_then(|s| s.phase)
        .is_some_and(|phase| phase == "Running")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn get_logs_and_at_logs_read_live_pods_bounded_and_redacted() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let pods = Api::<Pod>::namespaced(client.clone(), ns.name());
    for name in ["echo-a", "echo-b"] {
        pods.create(&PostParams::default(), &echo_pod(name))
            .await
            .expect("create the pod");
    }
    for name in ["echo-a", "echo-b"] {
        eventually(
            "the pod to run",
            || String::new(),
            || async { running(&pods, name).await },
        )
        .await;
    }
    // Let the pods write a few lines.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let spawner: Arc<dyn Spawner> = Arc::new(|task: BoxFuture<'static, ()>| {
        tokio::spawn(task);
    });
    let service = Arc::new(LogService::new(
        LogRuntime {
            spawner,
            clock: Arc::new(TokioClock),
        },
        LogConfig::default(),
    ));
    let resources = KubeResources::new(client.clone(), KubeDiscovery::new(client.clone()));
    let clusters = Arc::new(Fixed(LogCluster {
        id: catalog_entry(&kind.context).cluster,
        title: kind.context.to_string(),
        ports: AggregatePorts {
            logs: Arc::new(KubeLogs::new(client.clone())),
            resources: Arc::new(resources),
        },
    }));
    let tools = ToolRegistry::new();
    GetLogsTool::register(&tools, clusters.clone(), service.clone()).expect("register");
    let contexts = ContextRegistry::new();
    contexts
        .register(Arc::new(LogContextProvider::new(clusters, service)))
        .expect("register");
    let ctx = ToolContext::agent();

    // A pod read: the newest lines, ended by itself although the pod keeps writing.
    let started = Instant::now();
    let out = tools
        .invoke(
            "k8s.get_logs",
            json!({"pod": "echo-a", "namespace": ns.name(), "tail": 5}),
            &ctx,
        )
        .await
        .expect("a valid call");
    assert!(!out.is_error, "{out:?}");
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "it must not follow"
    );
    let text = out.content[0].as_text().unwrap();
    let lines: Vec<_> = text.lines().filter(|l| !l.starts_with("note:")).collect();
    assert_eq!(lines.len(), 5, "{text}");
    assert!(
        lines.iter().all(|l| l.contains("echo-a/main echo-a line ")),
        "{text}"
    );
    assert_eq!(out.structured.as_ref().unwrap()["lines"], 5);

    // The token the pod printed is masked; the rest of the line stays.
    let out = tools
        .invoke(
            "k8s.get_logs",
            json!({"pod": "echo-a", "namespace": ns.name(), "since": "1h", "grep": "upstream"}),
            &ctx,
        )
        .await
        .expect("a valid call");
    let text = out.content[0].as_text().unwrap();
    assert!(text.contains("calling upstream Authorization:"), "{text}");
    assert!(
        !text.contains(SECRET) && !text.contains("abcdefghijklmnopqrstuv"),
        "{text}"
    );

    // A selector reads every pod it picks, merged and attributed.
    let out = tools
        .invoke(
            "k8s.get_logs",
            json!({"selector": LABEL, "namespace": ns.name(), "grep": "line [0-9]$", "tail": 40}),
            &ctx,
        )
        .await
        .expect("a valid call");
    assert!(!out.is_error, "{out:?}");
    let text = out.content[0].as_text().unwrap();
    assert!(
        text.contains("echo-a/main") && text.contains("echo-b/main"),
        "{text}"
    );
    assert_eq!(out.structured.as_ref().unwrap()["streams"], 2);
    let stamps: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("note:"))
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert!(
        stamps.windows(2).all(|w| w[0] <= w[1]),
        "merged by server time\n{text}"
    );

    // `@logs` reads the same pod with `--since`, as a block with its source.
    let scope = ContextScope::new().with_default_namespace(ns.name());
    let blocks = contexts
        .resolve_text("@logs/echo-b/--since/1h/--tail=3", &scope)
        .await
        .expect("the mention resolves");
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].title.contains("since 1h"), "{}", blocks[0].title);
    assert!(
        blocks[0].body.contains("# source: echo-b"),
        "{}",
        blocks[0].body
    );
    assert!(!blocks[0].body.contains(SECRET));

    // A pod that does not exist is an error the model sees, not a failed call.
    let out = tools
        .invoke(
            "k8s.get_logs",
            json!({"pod": "ghost", "namespace": ns.name()}),
            &ctx,
        )
        .await
        .expect("the call itself is valid");
    assert!(out.is_error);
    assert!(out.content[0].as_text().unwrap().contains("ghost"));
}
