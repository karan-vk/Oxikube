//! Kind integration for E17-S09: a `TokenRequest` posted through `ResourceWriter::create_subresource`
//! (`serviceaccounts/<name>/token`), as the app's `serviceaccount::GenerateKubeconfig` posts it.
//! Needs `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
//!
//! The account lives in the test's own `oxi-test-<rand>` namespace. Tokens are secrets: nothing
//! here prints one.
#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;
use oxikube_ports::{ResourcePort, Subresource, WriteOptions};
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

use common::TestServiceAccount;
use common::resources::adapter;

fn account_gvk() -> Gvk {
    Gvk::new("", "v1", "ServiceAccount")
}

fn request(seconds: u32) -> Value {
    json!({
        "apiVersion": "authentication.k8s.io/v1", "kind": "TokenRequest",
        "spec": {"expirationSeconds": seconds},
    })
}

#[tokio::test]
async fn a_token_request_returns_a_token_and_the_expiry_the_server_chose() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let port: Arc<dyn ResourcePort> = Arc::new(adapter(&client));
    drop(TestServiceAccount::create(&client, ns.name(), "viewer", Vec::new()).await);
    let token = Subresource::Other("token".into());

    let response = port
        .create_subresource(
            &account_gvk(),
            Some(ns.name()),
            "viewer",
            &token,
            &request(900),
            &WriteOptions::default(),
        )
        .await
        .expect("a TokenRequest for an existing account");
    let issued = response["status"]["token"].as_str().expect("a token");
    assert!(issued.starts_with("eyJ"), "a JWT");
    assert!(
        response["status"]["expirationTimestamp"].is_string(),
        "the server says when it expires"
    );

    // Audiences are passed on and recorded in the request's spec echo.
    let response = port
        .create_subresource(
            &account_gvk(),
            Some(ns.name()),
            "viewer",
            &token,
            &json!({
                "apiVersion": "authentication.k8s.io/v1", "kind": "TokenRequest",
                "spec": {"expirationSeconds": 600, "audiences": ["https://oxikube.example"]},
            }),
            &WriteOptions::default(),
        )
        .await
        .expect("a TokenRequest with an audience");
    assert_eq!(
        response["spec"]["audiences"],
        json!(["https://oxikube.example"])
    );
}

#[tokio::test]
async fn a_missing_account_is_not_found_and_a_lifetime_under_ten_minutes_is_refused() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let port: Arc<dyn ResourcePort> = Arc::new(adapter(&client));
    drop(TestServiceAccount::create(&client, ns.name(), "viewer", Vec::new()).await);
    let token = Subresource::Other("token".into());

    let missing = port
        .create_subresource(
            &account_gvk(),
            Some(ns.name()),
            "ghost",
            &token,
            &request(900),
            &WriteOptions::default(),
        )
        .await
        .expect_err("no such account");
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");

    let short = port
        .create_subresource(
            &account_gvk(),
            Some(ns.name()),
            "viewer",
            &token,
            &request(60),
            &WriteOptions::default(),
        )
        .await
        .expect_err("the server's minimum is ten minutes");
    assert_eq!(short.kind(), ErrorKind::Validation, "{short}");
}
