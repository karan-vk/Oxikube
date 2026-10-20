//! Scenario D: an auto-start favourite saved by one run of the app starts again in the next.
//! The "restart" is a second stack (sessions, stores, manager, favourites) over a new
//! `SqliteState` on the same file; its session connects after the favourites' follower is
//! running, as at launch, and the forward comes back on its own.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Service;
use oxikube_app::portforward::PortForwardFavourites;
use oxikube_domain::portforward::ForwardPort;
use oxikube_state_sqlite::SqliteState;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::nginx;

use super::{Forwarding, create, is_active};
use crate::clock::TokioClock;
use crate::cluster::Kind;

fn favourites(forwarding: &Forwarding, state: Arc<SqliteState>) -> PortForwardFavourites {
    PortForwardFavourites::new(
        state,
        forwarding.manager.clone(),
        forwarding.spawner.clone(),
        Arc::new(TokioClock),
        Some(forwarding.notify.clone()),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_auto_start_favourite_is_restored_after_a_restart() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let client = kind.admin_client().await;
    create::<Deployment>(&client, ns.name(), nginx::deployment("web", "web", 1)).await;
    create::<Service>(&client, ns.name(), nginx::service("web", "web")).await;
    super::wait_ready_pod(&client, ns.name(), "web").await;
    let dir = tempfile::tempdir().expect("a temp dir");
    let db = dir.path().join("state.db");

    // First run: forward the Service, save it as an auto-start favourite, quit.
    let spec = {
        let first = Forwarding::connect(&kind).await;
        let state = Arc::new(SqliteState::open(&db).await.expect("open the state"));
        let spec = first.spec(
            "Service",
            ns.name(),
            "web",
            ForwardPort::Named("web".into()),
        );
        let id = first
            .manager
            .start_and_wait(spec.clone())
            .await
            .expect("starts")
            .id;
        let addr = first
            .wait(id, "active", is_active)
            .await
            .local_addr
            .unwrap();
        nginx::wait_welcome(addr, crate::DEADLINE).await;
        favourites(&first, state)
            .add(spec.clone(), Some("web".into()), true)
            .await
            .expect("save the favourite");
        spec
        // Dropping the stack stops the forward and closes the database.
    };

    // Second run: a fresh state over the same file and a session that is not connected yet.
    let second = Forwarding::open(&kind);
    let state = Arc::new(SqliteState::open(&db).await.expect("reopen the state"));
    let saved = favourites(&second, state);
    let listed = saved.list().await.expect("read the favourites");
    assert_eq!(listed.len(), 1, "the favourite survived: {listed:?}");
    assert!(listed[0].auto_start);
    assert!(
        second.manager.list().is_empty(),
        "nothing runs before the session is Ready"
    );
    let follower = tokio::spawn(saved.follow(&second.sessions));
    second
        .sessions
        .connect(&second.cluster)
        .await
        .expect("connect");

    let mut restored = None;
    crate::eventually(
        "the favourite to be forwarded again",
        || format!("{:?}", second.manager.list()),
        || {
            restored = second
                .manager
                .list()
                .into_iter()
                .find(|f| f.spec.target == spec.target && is_active(f));
            std::future::ready(restored.is_some())
        },
    )
    .await;
    let info = restored.expect("restored");
    assert_eq!(info.spec.remote_port, spec.remote_port);
    nginx::wait_welcome(info.local_addr.expect("bound"), crate::DEADLINE).await;
    follower.abort();
}
