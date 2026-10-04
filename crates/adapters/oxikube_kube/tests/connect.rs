//! Kind integration: connecting to several contexts at once through the `ClientPool`
//! (E03-S09; pool from E03-S03, loader from E03-S01). Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::Namespace;
use kube::Api;
use kube::api::ListParams;
use oxikube_domain::ids::ContextName;
use oxikube_kube::ClientPool;
use oxikube_testkit::integration::TestNamespace;

use common::{TestServiceAccount, whoami};

/// What one context reached: its client, the server version and who it is.
struct Connected {
    client: Arc<kube::Client>,
    version: String,
    username: String,
    time_to_client: Duration,
    time_to_version: Duration,
}

async fn connect(pool: &ClientPool, context: &ContextName) -> Connected {
    let started = Instant::now();
    let client = pool
        .get(context)
        .await
        .unwrap_or_else(|e| panic!("{context}: {e}"));
    let time_to_client = started.elapsed();
    let version = client
        .apiserver_version()
        .await
        .unwrap_or_else(|e| panic!("{context}: /version: {e}"));
    let time_to_version = started.elapsed();
    let username = whoami(&client)
        .await
        .unwrap_or_else(|e| panic!("{context}: whoami: {e}"));
    Connected {
        client,
        version: version.git_version,
        username,
        time_to_client,
        time_to_version,
    }
}

#[tokio::test]
async fn two_contexts_connect_concurrently() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let admin = kind.admin_client().await;
    let viewer = TestServiceAccount::create(&admin, ns.name(), "oxi-second-user", Vec::new()).await;

    // One kind cluster, two contexts with different names and users.
    let second = ContextName::from("oxi-second");
    let pool = kind.pool(kind.with_token_context(second.as_str(), &viewer.token));

    let (first, other) = tokio::join!(connect(&pool, &kind.context), connect(&pool, &second));
    eprintln!(
        "connect `{}`: client {:?}, /version {:?}; `{second}`: client {:?}, /version {:?}",
        kind.context,
        first.time_to_client,
        first.time_to_version,
        other.time_to_client,
        other.time_to_version
    );

    assert!(
        !Arc::ptr_eq(&first.client, &other.client),
        "one client per context"
    );
    assert_eq!(
        first.version, other.version,
        "same cluster behind both contexts"
    );
    assert_eq!(other.username, viewer.username);
    assert_ne!(
        first.username, other.username,
        "each context has its own user"
    );
    assert_eq!(pool.len(), 2);

    // The pool hands the same clients out again.
    assert!(Arc::ptr_eq(
        &first.client,
        &pool.get(&kind.context).await.expect("reuse")
    ));

    // The admin context sees the cluster, including this test's namespace.
    let names: Vec<String> = Api::<Namespace>::all((*first.client).clone())
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
