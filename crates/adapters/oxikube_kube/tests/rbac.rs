//! Kind integration: a restricted service account gets `Forbidden`, not a crash
//! (E03-S09; error classification from E03-S04). Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use k8s_openapi::api::core::v1::{Pod, Secret};
use k8s_openapi::api::rbac::v1::PolicyRule;
use kube::Api;
use kube::api::ListParams;
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::ContextName;
use oxikube_kube::auth::classify;
use oxikube_testkit::integration::TestNamespace;

use common::{DEADLINE, TestServiceAccount, wait_until, whoami};

#[tokio::test]
async fn restricted_account_gets_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    let pods_only = PolicyRule {
        api_groups: Some(vec![String::new()]),
        resources: Some(vec!["pods".into()]),
        verbs: vec!["get".into(), "list".into()],
        ..PolicyRule::default()
    };
    let account =
        TestServiceAccount::create(&admin, ns.name(), "oxi-restricted", vec![pods_only]).await;

    let restricted = ContextName::from("oxi-restricted");
    let pool = kind.pool(kind.with_token_context(restricted.as_str(), &account.token));
    let client = pool.get(&restricted).await.expect("restricted client");
    assert_eq!(whoami(&client).await.expect("whoami"), account.username);

    // Allowed: pods in the test namespace. RBAC takes a moment to apply, so poll.
    let pods = Api::<Pod>::namespaced((*client).clone(), ns.name());
    wait_until("the Role to allow listing pods", DEADLINE, || async {
        pods.list(&ListParams::default()).await.ok()
    })
    .await;

    // Not allowed: secrets in the same namespace, and pods anywhere else.
    let denied = [
        Api::<Secret>::namespaced((*client).clone(), ns.name())
            .list(&ListParams::default())
            .await
            .map(|_| ()),
        Api::<Pod>::namespaced((*client).clone(), "kube-system")
            .list(&ListParams::default())
            .await
            .map(|_| ()),
    ];
    for result in denied {
        let err = result.expect_err("the account may not do this");
        let classified = classify(&err);
        assert_eq!(classified.kind(), ErrorKind::Forbidden, "{classified:?}");
        assert!(!classified.is_retryable());
        // The apiserver names the identity and the resource; never the credential.
        assert!(
            classified.message().contains("oxi-restricted"),
            "{classified}"
        );
        assert!(!format!("{classified:?}").contains(&account.token));
    }
}
