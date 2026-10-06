//! Column provider parity (E07-S02): the columns we compute agree with the ones the API server
//! computes, on the real cluster.
//!
//! * Core: `Ready`, `Status`, `Restarts`, `Node` and `IP` of `CoreColumns` on pods read through the
//!   reflector port equal the server's Table cells for the same pods (the fixture pods of
//!   `cargo xtask kind-up`, whatever states they are in, plus pending pods of this test's own).
//! * Table: `TableColumns` on the sample `Widget` CRD shows the CRD's printer columns, flags the
//!   `priority: 1` one as wide, and maps every cell as the server rendered it.
//!
//! Read-only on the shared fixtures; the pods it creates live in its own `oxi-test-<rand>`
//! namespace and name a scheduler nobody runs.

use jiff::Timestamp;
use oxikube_app::columns::{ColumnId, ColumnProvider, CoreColumns, TableColumns};
use oxikube_app::store::{StoreObject, TableObject};
use oxikube_domain::Capabilities;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_kube::{KubeDiscovery, KubeResources};
use oxikube_ports::{ListOptions, ResourceReader, Table, TableFeedPort, TableOptions};
use oxikube_testkit::integration::TestNamespace;
use serde_json::Value;

use crate::cluster::{Kind, create_pods};

const FIXTURES: &str = "oxikube-fixtures";

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn widget_gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

/// The text the server printed in `table`'s `column` for `name`.
fn server_text(table: &Table, name: &str, column: &str) -> String {
    let i = table
        .columns
        .iter()
        .position(|c| c.name == column)
        .unwrap_or_else(|| panic!("the server table has no `{column}` column"));
    let row = table
        .rows
        .iter()
        .find(|r| r.cells.first().and_then(Value::as_str) == Some(name))
        .unwrap_or_else(|| panic!("no server row for `{name}`"));
    match &row.cells[i] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Compares our pod cells with the server's for every pod in `namespace`; returns how many.
async fn assert_pod_parity(reader: &KubeResources, namespace: &str) -> usize {
    let pods = reader
        .list(&pod_gvk(), Some(namespace), &ListOptions::default())
        .await
        .expect("list pods")
        .items;
    let table = reader
        .list_table(&pod_gvk(), Some(namespace), &TableOptions::default())
        .await
        .expect("list pods as a table");
    let provider = CoreColumns::new();
    let now = Timestamp::now();
    for pod in &pods {
        let ours = |id: &str| provider.resource_cell(pod, &ColumnId::new(id), now);
        let name = &*pod.meta.name;
        for (id, column) in [("ready", "Ready"), ("status", "Status")] {
            assert_eq!(
                ours(id).display(),
                server_text(&table, name, column),
                "pod {namespace}/{name}: {column}"
            );
        }
        // The server appends `(5m ago)` since the last restart; only the count is comparable.
        let restarts = ours("restarts").display().to_owned();
        let theirs = server_text(&table, name, "Restarts");
        assert_eq!(
            restarts.split(' ').next(),
            theirs.split(' ').next(),
            "pod {namespace}/{name}: Restarts"
        );
        // The Table hides `<none>` placeholders as text; a pod without a value shows nothing here.
        for (id, column) in [("ip", "IP"), ("node", "Node")] {
            let theirs = server_text(&table, name, column);
            let theirs = if theirs == "<none>" {
                ""
            } else {
                theirs.as_str()
            };
            assert_eq!(
                ours(id).display(),
                theirs,
                "pod {namespace}/{name}: {column}"
            );
        }
    }
    pods.len()
}

async fn resources(kind: &Kind) -> KubeResources {
    let client = kind.admin_client().await;
    KubeResources::new(client.clone(), KubeDiscovery::new(client))
}

#[tokio::test]
async fn core_pod_columns_match_the_server_table() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let reader = resources(&kind).await;

    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    create_pods(
        &kind.admin_client().await,
        ns.name(),
        "columns",
        "pending",
        3,
    )
    .await;
    let own = assert_pod_parity(&reader, ns.name()).await;
    assert_eq!(own, 3);

    // The fixture pods cover Running, ImagePullBackOff, Completed and Pending; when the
    // fixtures are applied they must agree too.
    let shared = assert_pod_parity(&reader, FIXTURES).await;
    eprintln!("checked {own} own pods and {shared} fixture pods against the server table");
}

#[tokio::test]
async fn table_columns_show_a_crds_printer_columns() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let reader = resources(&kind).await;
    let table = reader
        .list_table(&widget_gvk(), Some(FIXTURES), &TableOptions::default())
        .await
        .expect("list widgets as a table");
    if table.rows.is_empty() {
        eprintln!("no Widget fixtures in `{FIXTURES}`; run `cargo xtask kind-up`");
        return;
    }
    let provider = TableColumns::new(&table.columns, table.source, Scope::Namespaced);
    let columns = provider.columns(&widget_gvk(), Capabilities::empty());

    let titles: Vec<&str> = columns.iter().map(|c| &*c.title).collect();
    assert_eq!(
        titles,
        [
            "Name",
            "Namespace",
            "Size",
            "Replicas",
            "Phase",
            "Age",
            "Owner"
        ],
        "printer columns plus the synthetic namespace column"
    );
    let wide: Vec<&str> = columns
        .iter()
        .filter(|c| c.wide)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(wide, ["namespace", "owner"], "`priority: 1` is wide");

    let now = Timestamp::now();
    for row in &table.rows {
        let meta = row.meta.clone().expect("rows carry metadata");
        let name = meta.name.to_string();
        let object = StoreObject::Row(TableObject {
            meta,
            cells: row.cells.clone(),
            object: None,
        });
        for (id, column) in [
            ("size", "Size"),
            ("owner", "Owner"),
            ("replicas", "Replicas"),
        ] {
            assert_eq!(
                provider.cell(&object, &ColumnId::new(id), now).display(),
                server_text(&table, &name, column),
                "widget {name}: {column}"
            );
        }
        assert_eq!(
            provider
                .cell(&object, &ColumnId::new("namespace"), now)
                .display(),
            FIXTURES
        );
    }
}
