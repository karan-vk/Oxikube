//! kv and typed-table behaviour.

use futures::executor::block_on;
use oxikube_ports::{StatePort, StatePortExt};
use serde_json::json;

use super::{Fixture, key, table};

#[test]
fn kv_round_trips_replaces_and_deletes() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        assert_eq!(state.kv_get(&key("a")).await.unwrap(), None);
        state
            .kv_set(&key("a"), json!({"n": 1, "s": "héllo"}))
            .await
            .unwrap();
        assert_eq!(
            state.kv_get(&key("a")).await.unwrap(),
            Some(json!({"n": 1, "s": "héllo"}))
        );
        state
            .kv_set(&key("a"), json!([1, 2.5, null]))
            .await
            .unwrap();
        assert_eq!(
            state.kv_get(&key("a")).await.unwrap(),
            Some(json!([1, 2.5, null]))
        );
        assert!(state.kv_delete(&key("a")).await.unwrap());
        assert!(!state.kv_delete(&key("a")).await.unwrap());
        assert_eq!(state.kv_get(&key("a")).await.unwrap(), None);
    });
}

#[test]
fn kv_survives_a_reopen() {
    let fx = Fixture::new();
    block_on(async {
        let state = fx.open_async().await;
        state
            .kv_set(&key("window/layout"), json!({"w": 800}))
            .await
            .unwrap();
        state
            .table_put(&table("favourites"), &key("pod/web"), json!(true))
            .await
            .unwrap();
        drop(state);

        let state = fx.open_async().await;
        assert!(state.recovery().is_none());
        assert_eq!(
            state.kv_get(&key("window/layout")).await.unwrap(),
            Some(json!({"w": 800}))
        );
        assert_eq!(
            state
                .table_get(&table("favourites"), &key("pod/web"))
                .await
                .unwrap(),
            Some(json!(true))
        );
    });
}

#[test]
fn kv_list_filters_by_prefix_in_key_order() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        for k in [
            "recent/b", "recent/a", "recent_x", "recentAx", "other", "recent/c",
        ] {
            state.kv_set(&key(k), json!(k)).await.unwrap();
        }
        let keys = |rows: Vec<(oxikube_ports::StateKey, serde_json::Value)>| {
            rows.into_iter()
                .map(|(k, _)| k.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            keys(state.kv_list("recent/").await.unwrap()),
            ["recent/a", "recent/b", "recent/c"]
        );
        assert_eq!(
            keys(state.kv_list("recent").await.unwrap()),
            ["recent/a", "recent/b", "recent/c", "recentAx", "recent_x"]
        );
        // `_` and `%` are literal, not LIKE wildcards.
        assert_eq!(keys(state.kv_list("recent_").await.unwrap()), ["recent_x"]);
        assert!(state.kv_list("100%").await.unwrap().is_empty());
        assert_eq!(state.kv_list("").await.unwrap().len(), 6);
        assert!(state.kv_list("nothing").await.unwrap().is_empty());
    });
}

#[test]
fn typed_tables_are_isolated_from_kv_and_each_other() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        let (fav, fwd) = (table("favourites"), table("port_forwards"));
        state.kv_set(&key("x"), json!("kv")).await.unwrap();
        state
            .table_put(&fav, &key("x"), json!("fav"))
            .await
            .unwrap();
        state
            .table_put(&fwd, &key("x"), json!("fwd"))
            .await
            .unwrap();
        state
            .table_put(&fav, &key("y"), json!("fav-y"))
            .await
            .unwrap();

        assert_eq!(state.kv_get(&key("x")).await.unwrap(), Some(json!("kv")));
        assert_eq!(
            state.table_get(&fav, &key("x")).await.unwrap(),
            Some(json!("fav"))
        );
        assert_eq!(
            state.table_get(&fwd, &key("x")).await.unwrap(),
            Some(json!("fwd"))
        );
        assert_eq!(
            state.kv_list("").await.unwrap().len(),
            1,
            "tables are not in kv"
        );

        let rows = state.table_list(&fav, None).await.unwrap();
        assert_eq!(
            rows.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["x", "y"]
        );
        assert_eq!(state.table_list(&fav, Some(1)).await.unwrap().len(), 1);
        assert!(
            state
                .table_list(&table("empty"), None)
                .await
                .unwrap()
                .is_empty()
        );

        assert!(state.table_delete(&fav, &key("x")).await.unwrap());
        assert!(!state.table_delete(&fav, &key("x")).await.unwrap());
        assert_eq!(
            state.table_get(&fwd, &key("x")).await.unwrap(),
            Some(json!("fwd"))
        );
    });
}

#[test]
fn typed_helpers_work_through_a_dyn_port() {
    let fx = Fixture::new();
    let state: std::sync::Arc<dyn StatePort> = std::sync::Arc::new(fx.open());
    block_on(async {
        state.kv_set_as(&key("n"), &vec![1u32, 2, 3]).await.unwrap();
        assert_eq!(
            state.kv_get_as::<Vec<u32>>(&key("n")).await.unwrap(),
            Some(vec![1, 2, 3])
        );
        let err = state.kv_get_as::<String>(&key("n")).await.unwrap_err();
        assert_eq!(err.kind(), oxikube_domain::ErrorKind::Validation);
    });
}

#[test]
fn concurrent_calls_all_land_and_reads_see_earlier_writes() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        let keys: Vec<_> = (0..50).map(|i| key(&format!("k{i:02}"))).collect();
        let writes = keys
            .iter()
            .enumerate()
            .map(|(i, k)| state.kv_set(k, json!(i)));
        futures::future::try_join_all(writes).await.unwrap();
        assert_eq!(state.kv_list("k").await.unwrap().len(), 50);
    });
}
