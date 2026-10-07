//! Kind integration: a refused websocket upgrade (pod exec, attach, port-forward) is
//! classified by the shared [`classify`] as `Forbidden` / `Auth` / `NotFound`, not as a
//! retryable `Network` error (E03-F448). Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use k8s_openapi::api::core::v1::Pod;
use kube::api::AttachParams;
use kube::{Api, Client};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_kube::auth::{CredentialRefresh, classify, classify_with};
use oxikube_testkit::integration::TestNamespace;

use common::TestServiceAccount;
use common::exec::create_sleeper;
use common::portforward::wait_ready;

/// The error from opening each streaming subresource of `ns/pod` with `client`.
async fn upgrade_errors(client: &Client, ns: &str, pod: &str) -> Vec<(&'static str, kube::Error)> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    let params = AttachParams::default().stderr(false);
    let mut out = Vec::new();
    if let Err(e) = pods.exec(pod, ["true"], &params).await {
        out.push(("exec", e));
    }
    if let Err(e) = pods.attach(pod, &params).await {
        out.push(("attach", e));
    }
    if let Err(e) = pods.portforward(pod, &[80]).await {
        out.push(("portforward", e));
    }
    out
}

fn assert_all(
    errors: &[(&str, kube::Error)],
    kind: ErrorKind,
    retryable: bool,
    classify: impl Fn(&kube::Error) -> OxiError,
) {
    assert_eq!(
        errors.len(),
        3,
        "exec, attach and portforward all fail: {errors:?}"
    );
    for (what, err) in errors {
        assert!(
            matches!(err, kube::Error::UpgradeConnection(_)),
            "{what}: {err:?}"
        );
        let classified = classify(err);
        assert_eq!(classified.kind(), kind, "{what}: {classified:?}");
        assert_eq!(
            classified.is_retryable(),
            retryable,
            "{what}: {classified:?}"
        );
    }
}

#[tokio::test]
async fn upgrade_without_rbac_is_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    create_sleeper(&admin, ns.name(), "box").await;
    wait_ready(&admin, ns.name(), "box").await;

    // No Role at all: every request is forbidden, including the streaming ones.
    let account = TestServiceAccount::create(&admin, ns.name(), "no-rbac", Vec::new()).await;
    let name = "no-rbac";
    let pool = kind.pool(kind.with_token_context(name, &account.token));
    let client = pool.get(&name.into()).await.expect("no-rbac client");

    let errors = upgrade_errors(&client, ns.name(), "box").await;
    assert_all(&errors, ErrorKind::Forbidden, false, classify);
    for (_, err) in &errors {
        let text = format!("{:?}", classify(err));
        assert!(!text.contains(&account.token), "token leaked: {text}");
    }
}

#[tokio::test]
async fn upgrade_with_a_rejected_token_is_auth() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    create_sleeper(&admin, ns.name(), "box").await;
    wait_ready(&admin, ns.name(), "box").await;

    let name = "bad-token";
    let pool = kind.pool(kind.with_token_context(name, "not-a-valid-token"));
    let client = pool.get(&name.into()).await.expect("client");
    let errors = upgrade_errors(&client, ns.name(), "box").await;

    // A refreshable credential may recover on retry; a static one cannot.
    assert_all(&errors, ErrorKind::Auth, true, |e| {
        classify_with(e, CredentialRefresh::Refreshable)
    });
    assert_all(&errors, ErrorKind::Auth, false, |e| {
        classify_with(e, CredentialRefresh::Static)
    });
}

#[tokio::test]
async fn upgrade_to_a_missing_pod_is_not_found() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;

    let errors = upgrade_errors(&admin, ns.name(), "ghost").await;
    assert_all(&errors, ErrorKind::NotFound, false, classify);
}
