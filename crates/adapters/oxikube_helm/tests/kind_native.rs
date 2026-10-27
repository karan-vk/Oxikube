//! The native release reader against kind, through the real kube resource adapter
//! (feature `integration`; skipped when `OXIKUBE_TEST_CONTEXT` is unset).
//!
//! Two sources, each in its own `oxi-test-<rand>` namespace:
//! * the recorded helm 4.3.1 objects (both drivers), created with `kubectl`: proves the
//!   Secret base64 layering survives the API server and the kube adapter;
//! * when a `helm` binary is on `PATH`, releases made by it from `tests/fixtures/chart`
//!   (installed, upgraded, and one with the `configmap` driver). Without `helm` that half is
//!   skipped with a notice; the full CLI suite is E16-S12.

#![cfg(feature = "integration")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_helm::NativeHelmReader;
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_kube::{
    ClientPool, ContextDefinition, KubeClientFactory, KubeDiscovery, KubeResources, PoolConfig,
    ProxyEnv, SystemClock,
};
use oxikube_ports::helm::{HelmPort, ReleaseRef, ReleaseStatus};
use oxikube_testkit::integration::{TestNamespace, events, test_context};
use serde_json::{Value, json};

/// A reader for the kind context, over the real resource adapter.
async fn native_reader(context: &str) -> (NativeHelmReader, ClusterId) {
    let home = std::env::var_os("HOME").map(|h| default_kubeconfig_path(&PathBuf::from(h)));
    let loaded = load_local_kubeconfig(
        std::env::var_os("KUBECONFIG"),
        home,
        Strictness::RequireUsable,
    )
    .await
    .expect("load the local kubeconfig");
    let name = ContextName::from(context.to_owned());
    let definition = ContextDefinition::from_kubeconfig(&loaded.merged, &name)
        .unwrap_or_else(|| panic!("context `{context}` is not in the kubeconfig"));
    let pool = ClientPool::with_parts(
        definition.kubeconfig().clone(),
        PoolConfig::default(),
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    let client = (*pool.get(&name).await.expect("kind client")).clone();
    let resources = KubeResources::new(client.clone(), KubeDiscovery::new(client));
    let cluster = ClusterId::new("kind-test", &name);
    (
        NativeHelmReader::new(cluster.clone(), Arc::new(resources)),
        cluster,
    )
}

fn recorded() -> Vec<Value> {
    [
        include_str!("fixtures/helm4.3.1-secret-web.v1.json"),
        include_str!("fixtures/helm4.3.1-secret-web.v2.json"),
        include_str!("fixtures/helm4.3.1-secret-web.v3.json"),
        include_str!("fixtures/helm4.3.1-configmap-web.cm-app.v1.json"),
        include_str!("fixtures/helm4.3.1-configmap-web.cm-app.v2.json"),
    ]
    .into_iter()
    .map(|text| {
        let mut object: Value = serde_json::from_str(text).expect("recorded fixture is valid JSON");
        let metadata = object["metadata"]
            .as_object_mut()
            .expect("recorded fixture metadata is a JSON object");
        for server_field in ["namespace", "uid", "resourceVersion", "creationTimestamp"] {
            metadata.remove(server_field);
        }
        object
    })
    .collect()
}

fn in_namespace(namespace: &str) -> NamespaceSelection {
    NamespaceSelection::from_names([namespace])
}

#[tokio::test]
async fn reads_recorded_release_objects_through_the_kube_adapter() {
    let Some(context) = test_context() else {
        return;
    };
    let ns = TestNamespace::create(&context).expect("namespace");
    for object in recorded() {
        events::create(&context, ns.name(), &object).expect("create release object");
    }
    let (reader, cluster) = native_reader(&context).await;

    let releases = reader
        .list_releases(&cluster, &in_namespace(ns.name()))
        .await
        .unwrap();
    let rows: Vec<_> = releases
        .iter()
        .map(|r| {
            (
                r.release.name.as_str(),
                r.revision,
                r.status,
                r.chart.to_string(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("web", 3, ReleaseStatus::Failed, "demo-1.2.3".into()),
            (
                "web.cm-app",
                2,
                ReleaseStatus::Deployed,
                "demo-1.2.3".into()
            ),
        ]
    );
    let web = ReleaseRef::new(cluster.clone(), ns.name(), "web");
    let history = reader.history(&web).await.unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[2].description, "Install complete");
    let values = reader.values(&web, Some(1), false).await.unwrap();
    assert_eq!(*values.as_json(), json!({"greeting": "hi"}));
    let manifest = reader.manifest(&web, Some(2)).await.unwrap();
    assert!(manifest.contains("name: web-cfg"), "{manifest}");
}

fn helm_available() -> bool {
    Command::new("helm")
        .arg("version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn helm(context: &str, namespace: &str, driver: &str, args: &[&str]) {
    let out = Command::new("helm")
        .args(["--kube-context", context, "-n", namespace])
        .args(args)
        .env("HELM_DRIVER", driver)
        .output()
        .expect("run helm");
    assert!(
        out.status.success(),
        "helm {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn reads_releases_made_by_the_helm_cli() {
    let Some(context) = test_context() else {
        return;
    };
    if !helm_available() {
        eprintln!(
            "skipping: no `helm` binary on PATH (the native reader needs none; this test does)"
        );
        return;
    }
    let ns = TestNamespace::create(&context).expect("namespace");
    let chart = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chart");
    let chart = chart.to_str().unwrap();
    let suffix = &ns.name()[ns.name().len() - 8..];
    let (app, cm_app) = (format!("app-{suffix}"), format!("cm.app-{suffix}"));
    helm(&context, ns.name(), "secret", &["install", &app, chart]);
    helm(
        &context,
        ns.name(),
        "secret",
        &["upgrade", &app, chart, "--set", "image.tag=9.9"],
    );
    helm(
        &context,
        ns.name(),
        "configmap",
        &["install", &cm_app, chart],
    );

    let (reader, cluster) = native_reader(&context).await;
    let releases = reader
        .list_releases(&cluster, &in_namespace(ns.name()))
        .await
        .unwrap();
    // Sorted by name: `app-…` before `cm.app-…`.
    let rows: Vec<_> = releases
        .iter()
        .map(|r| (r.release.name.clone(), r.revision, r.status))
        .collect();
    let expected = vec![
        (app.clone(), 2, ReleaseStatus::Deployed),
        (cm_app.clone(), 1, ReleaseStatus::Deployed),
    ];
    assert_eq!(rows, expected);

    let app_ref = ReleaseRef::new(cluster.clone(), ns.name(), &app);
    let statuses: Vec<_> = reader
        .history(&app_ref)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.revision, r.status))
        .collect();
    assert_eq!(
        statuses,
        vec![(2, ReleaseStatus::Deployed), (1, ReleaseStatus::Superseded)]
    );
    let all = reader.values(&app_ref, None, true).await.unwrap();
    assert_eq!(all.get("image.tag"), Some(&json!("9.9")));
    assert_eq!(all.get("greeting"), Some(&json!("hello")));
    let notes = reader.notes(&app_ref, None).await.unwrap().unwrap();
    assert_eq!(notes.trim(), format!("Thanks for installing {app}."));
}
