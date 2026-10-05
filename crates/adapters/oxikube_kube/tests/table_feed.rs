//! Kind integration for E04-S04: the Table API feed on the sample `Widget` CRD (printer
//! columns, `-o wide` priority) and on pods (`kubectl get pods -o wide` parity), plus a live
//! feed seeing create, update and delete. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use futures::StreamExt;
use kube::api::{
    Api, ApiResource, DeleteParams, DynamicObject, GroupVersionKind, Patch, PatchParams, PostParams,
};
use oxikube_domain::ids::Gvk;
use oxikube_ports::{
    Delta, IncludeObject, Table, TableBatch, TableFeed, TableFeedPort, TableOptions, TableRow,
    TableSource,
};
use oxikube_testkit::integration::TestNamespace;
use serde_json::{Value, json};

use common::resources::{adapter, create_pods, pending_pod};
use common::table::{kubectl_get, render_cell};

fn widget_gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

fn widgets_api(client: &kube::Client, namespace: &str) -> Api<DynamicObject> {
    let gvk = GroupVersionKind::gvk("test.oxikube.dev", "v1", "Widget");
    let resource = ApiResource::from_gvk_with_plural(&gvk, "widgets");
    Api::namespaced_with(client.clone(), namespace, &resource)
}

fn widget(name: &str, size: &str, replicas: i64, owner: &str) -> DynamicObject {
    serde_json::from_value(json!({
        "apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
        "metadata": {"name": name},
        "spec": {"size": size, "replicas": replicas, "owner": owner},
    }))
    .expect("widget json")
}

/// Every non-`Age` cell of `table` equals what `kubectl get -o wide` prints for its row, and
/// the headers are the same (kubectl upper-cases them).
fn assert_kubectl_parity(table: &Table, kubectl: &(Vec<String>, Vec<Vec<String>>)) {
    let (headers, rows) = kubectl;
    let ours: Vec<String> = table
        .columns
        .iter()
        .map(|c| c.name.to_uppercase())
        .collect();
    assert_eq!(&ours, headers, "column headers");
    assert_eq!(table.rows.len(), rows.len(), "row count");
    for theirs in rows {
        let row = table
            .rows
            .iter()
            .find(|r| r.cells.first().and_then(Value::as_str) == Some(theirs[0].as_str()))
            .unwrap_or_else(|| panic!("no row named {}", theirs[0]));
        for (i, column) in table.columns.iter().enumerate() {
            if column.name == "Age" {
                continue; // relative time: rendered at different instants
            }
            assert_eq!(
                render_cell(&row.cells[i]),
                theirs[i],
                "row {} column {}",
                theirs[0],
                column.name
            );
        }
    }
}

#[tokio::test]
async fn crd_printer_columns_match_kubectl_get() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let api = widgets_api(&client, ns.name());
    for (name, size, replicas, owner) in
        [("w-1", "small", 1, "team-a"), ("w-2", "large", 7, "team-b")]
    {
        api.create(&PostParams::default(), &widget(name, size, replicas, owner))
            .await
            .expect("create widget");
    }

    let table = adapter(&client)
        .list_table(&widget_gvk(), Some(ns.name()), &TableOptions::default())
        .await
        .expect("list table");
    assert_eq!(table.source, TableSource::Server);
    let names: Vec<_> = table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Name", "Size", "Replicas", "Phase", "Age", "Owner"]);
    let default: Vec<_> = table
        .columns
        .iter()
        .filter(|c| c.is_default())
        .map(|c| c.name.to_uppercase())
        .collect();

    // `-o wide` shows every column; the default view only priority 0.
    let wide = kubectl_get(kind.context.as_str(), ns.name(), "widgets", true);
    assert_kubectl_parity(&table, &wide);
    let narrow = kubectl_get(kind.context.as_str(), ns.name(), "widgets", false);
    assert_eq!(narrow.0, default);

    for row in &table.rows {
        let meta = row.meta.as_ref().expect("includeObject=Metadata identity");
        assert!(meta.uid.is_some() && meta.resource_version.is_some());
        assert_eq!(meta.namespace.as_deref(), Some(ns.name()));
    }
}

#[tokio::test]
async fn pods_match_kubectl_get_pods_o_wide() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    create_pods(
        &client,
        ns.name(),
        vec![
            pending_pod("p-1", &[("app", "t")]),
            pending_pod("p-2", &[("app", "t")]),
        ],
    )
    .await;

    let table = adapter(&client)
        .list_table(
            &Gvk::new("", "v1", "Pod"),
            Some(ns.name()),
            &TableOptions::default(),
        )
        .await
        .expect("list pods table");
    let wide: Vec<_> = table
        .columns
        .iter()
        .filter(|c| !c.is_default())
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(wide, ["IP", "Node", "Nominated Node", "Readiness Gates"]);
    let kubectl = kubectl_get(kind.context.as_str(), ns.name(), "pods", true);
    assert_kubectl_parity(&table, &kubectl);
}

/// The next batch within the deadline.
async fn next_batch(feed: &mut TableFeed) -> TableBatch {
    tokio::time::timeout(common::DEADLINE, feed.next())
        .await
        .expect("no batch within the deadline")
        .expect("feed ended")
        .expect("feed error")
}

/// Reads batches until one holds a delta for `name` matching `want`.
async fn wait_for_delta(
    feed: &mut TableFeed,
    name: &str,
    want: impl Fn(&Delta<TableRow>) -> bool,
) -> TableRow {
    loop {
        let batch = next_batch(feed).await;
        assert!(batch.columns.is_none(), "no restart expected: {batch:?}");
        for delta in batch.rows.deltas {
            let row = match &delta {
                Delta::Applied(row) | Delta::Deleted(row) => row,
                Delta::Restarted(_) => panic!("unexpected restart"),
            };
            if row.meta.as_ref().is_some_and(|m| &*m.name == name) && want(&delta) {
                return row.clone();
            }
        }
    }
}

#[tokio::test]
async fn a_live_feed_sees_create_update_and_delete() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let api = widgets_api(&client, ns.name());
    api.create(
        &PostParams::default(),
        &widget("existing", "small", 1, "team-a"),
    )
    .await
    .expect("create widget");

    let mut feed = adapter(&client)
        .table_feed(&widget_gvk(), Some(ns.name()), &TableOptions::default())
        .await
        .expect("open feed");
    let first = next_batch(&mut feed).await;
    assert_eq!(first.columns.as_ref().map(|c| c.len()), Some(6));
    let Some(Delta::Restarted(rows)) = first.rows.deltas.first() else {
        panic!("first batch must restart");
    };
    assert_eq!(rows.len(), 1);

    api.create(
        &PostParams::default(),
        &widget("fresh", "medium", 2, "team-c"),
    )
    .await
    .expect("create widget");
    let added = wait_for_delta(&mut feed, "fresh", |d| matches!(d, Delta::Applied(_))).await;
    assert_eq!(added.cells[1], json!("medium"));
    assert_eq!(added.cells[2], json!(2));

    let patch = json!({"spec": {"replicas": 9}});
    api.patch("fresh", &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .expect("patch widget");
    let updated = wait_for_delta(
        &mut feed,
        "fresh",
        |d| matches!(d, Delta::Applied(row) if row.cells[2] == json!(9)),
    )
    .await;
    assert_ne!(
        updated.meta.as_ref().unwrap().resource_version,
        added.meta.as_ref().unwrap().resource_version
    );

    api.delete("fresh", &DeleteParams::default())
        .await
        .expect("delete widget");
    wait_for_delta(&mut feed, "fresh", |d| matches!(d, Delta::Deleted(_))).await;
}

#[tokio::test]
async fn whole_objects_and_paged_first_lists_work_against_the_server() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;
    let api = widgets_api(&client, ns.name());
    for i in 0..5 {
        api.create(
            &PostParams::default(),
            &widget(&format!("w-{i}"), "small", i, "team-a"),
        )
        .await
        .expect("create widget");
    }
    let options = TableOptions::default()
        .include_object(IncludeObject::Object)
        .list(oxikube_ports::ListOptions::default().limit(2));
    let mut feed = adapter(&client)
        .table_feed(&widget_gvk(), Some(ns.name()), &options)
        .await
        .expect("open feed");
    let first = next_batch(&mut feed).await;
    let Some(Delta::Restarted(rows)) = first.rows.deltas.first() else {
        panic!("first batch must restart");
    };
    assert_eq!(rows.len(), 5, "three pages of two make one restart");
    let object = rows[0].object.as_ref().expect("includeObject=Object");
    assert_eq!(object["kind"], json!("Widget"));
    assert_eq!(object["spec"]["size"], json!("small"));
}
