//! The SQLite store and `FakeStatePort` must agree: the fake stands in for it in every app test.

use futures::executor::block_on;
use oxikube_domain::audit::AuditOutcome::{Failed, Succeeded};
use oxikube_ports::{AuditQuery, StatePort};
use oxikube_testkit::fakes::FakeStatePort;
use serde_json::json;

use super::{Fixture, cluster, key, record, table, ts};

async fn script(state: &dyn StatePort) -> serde_json::Value {
    let fav = table("favourites");
    state.kv_set(&key("b"), json!(2)).await.unwrap();
    state.kv_set(&key("a"), json!(1)).await.unwrap();
    state.kv_set(&key("a/x"), json!(3)).await.unwrap();
    state.table_put(&fav, &key("z"), json!("z")).await.unwrap();
    state.table_put(&fav, &key("m"), json!("m")).await.unwrap();
    let del_kv = state.kv_delete(&key("b")).await.unwrap();
    let del_kv_again = state.kv_delete(&key("b")).await.unwrap();
    state
        .append_audit(&[
            record("prod", "pod::Delete", "2026-10-01T10:00:00Z", Succeeded),
            record("dev", "pod::Delete", "2026-10-01T10:00:00Z", Failed),
            record("prod", "workload::Scale", "2026-10-01T09:00:00Z", Succeeded),
        ])
        .await
        .unwrap();
    let audit = state
        .query_audit(&AuditQuery {
            cluster: Some(cluster("prod")),
            until: Some(ts("2026-10-01T10:00:01Z")),
            ..Default::default()
        })
        .await
        .unwrap();
    json!({
        "kv_a": state.kv_get(&key("a")).await.unwrap(),
        "kv_list": state.kv_list("a").await.unwrap().into_iter().map(|(k, v)| (k.as_str().to_owned(), v)).collect::<Vec<_>>(),
        "table_list": state.table_list(&fav, None).await.unwrap().into_iter().map(|(k, v)| (k.as_str().to_owned(), v)).collect::<Vec<_>>(),
        "table_limit": state.table_list(&fav, Some(1)).await.unwrap().len(),
        "deleted": [del_kv, del_kv_again],
        "audit": serde_json::to_value(audit).unwrap(),
    })
}

#[test]
fn sqlite_and_the_fake_agree() {
    let fx = Fixture::new();
    let sqlite = fx.open();
    let fake = FakeStatePort::new();
    block_on(async {
        assert_eq!(script(&sqlite).await, script(&fake).await);
    });
}
