//! Kind integration for E07-S10: the API server's `Warning:` response headers reach the
//! connection's `WarningPort`. A namespace labelled `pod-security.kubernetes.io/warn=restricted`
//! makes the server warn ("would violate PodSecurity") when a pod without a restricted security
//! context is created there; the test creates one through a pooled client and expects the warning
//! on the hub's port for the context. Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`;
//! skips cleanly otherwise.
//!
//! The pod is never scheduled (`pending_pod`) and lives in the test's own `oxi-test-<rand>`
//! namespace, which goes away with it.
#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use futures::StreamExt as _;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::Api;
use kube::api::{Patch, PatchParams, PostParams};
use oxikube_kube::warnings::WarningHub;
use oxikube_ports::WarningPort as _;
use oxikube_testkit::integration::TestNamespace;
use serde_json::json;

use common::resources::pending_pod;

#[tokio::test]
async fn the_servers_warning_header_reaches_the_warning_port() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;

    let namespaces: Api<Namespace> = Api::all((*client).clone());
    namespaces
        .patch(
            ns.name(),
            &PatchParams::default(),
            &Patch::Merge(&json!({
                "metadata": { "labels": { "pod-security.kubernetes.io/warn": "restricted" } }
            })),
        )
        .await
        .expect("label the namespace");

    // The pool built `client` with the warning layer; its warnings go to the hub under the
    // context name, which is where the connector reads them from.
    let mut warnings = WarningHub::global().port(&kind.context).subscribe();
    let pods: Api<Pod> = Api::namespaced((*client).clone(), ns.name());
    pods.create(&PostParams::default(), &pending_pod("warns", &[]))
        .await
        .expect("create the pod (a warning does not refuse it)");

    let warning = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let warning = warnings.next().await.expect("the port stays open");
            if warning.text.contains("PodSecurity") {
                return warning;
            }
        }
    })
    .await
    .expect("a PodSecurity warning within 10 s");
    assert_eq!(warning.code, 299);
    assert!(
        warning.text.contains("restricted"),
        "the text names the profile: {}",
        warning.text
    );
}
