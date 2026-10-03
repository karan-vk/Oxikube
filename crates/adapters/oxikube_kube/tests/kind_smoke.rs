//! Placeholder kind integration test (E01-S09): proves the workflow end to end until
//! E03-S09 replaces it. Skips cleanly when `OXIKUBE_TEST_CONTEXT` is unset.
#![cfg(feature = "integration")]

use k8s_openapi::api::core::v1::Namespace;
use kube::api::ListParams;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config};
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};

async fn client_for(context: &str) -> Client {
    ensure_kind_context(context).expect("kind context");
    let options = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..Default::default()
    };
    let kubeconfig = Kubeconfig::read().expect("read kubeconfig");
    let config = Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .expect("kubeconfig for context");
    Client::try_from(config).expect("client")
}

#[tokio::test]
async fn lists_namespaces_including_its_own() {
    let Some(ctx) = test_context() else { return };
    let ns = TestNamespace::create(&ctx).expect("create test namespace");
    let client = client_for(&ctx).await;

    let api: Api<Namespace> = Api::all(client);
    let names: Vec<String> = api
        .list(&ListParams::default())
        .await
        .expect("list namespaces")
        .items
        .into_iter()
        .filter_map(|n| n.metadata.name)
        .collect();

    assert!(names.iter().any(|n| n == "kube-system"), "{names:?}");
    assert!(names.iter().any(|n| n == ns.name()), "{names:?}");
}
