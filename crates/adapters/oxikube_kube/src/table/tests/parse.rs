//! `list_table`: request shape, recorded Table fixtures to port types, the plain-JSON
//! fallback, `includeObject` variants and error mapping.

use oxikube_domain::ErrorKind;
use oxikube_ports::{IncludeObject, ListOptions, TableFeedPort, TableOptions, TableSource};
use serde_json::json;

use super::harness::*;
use crate::fake_api::status_body;
use crate::is_list_expired;
use crate::table::{TABLE_ACCEPT, TableConfig};

fn names(columns: &[oxikube_ports::TableColumn]) -> Vec<&str> {
    columns.iter().map(|c| c.name.as_str()).collect()
}

#[tokio::test]
async fn a_crd_table_keeps_printer_columns_priorities_and_row_identity() {
    let api = server();
    api.reply(WIDGETS, 200, widgets_table());
    let table = adapter(&api, TableConfig::default())
        .list_table(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap();

    assert_eq!(table.source, TableSource::Server);
    assert_eq!(
        names(&table.columns),
        ["Name", "Size", "Replicas", "Phase", "Age", "Owner"]
    );
    let wide: Vec<_> = table.columns.iter().filter(|c| !c.is_default()).collect();
    assert_eq!(wide.len(), 1);
    assert_eq!(wide[0].name, "Owner");
    assert_eq!(table.columns[2].column_type, "integer");
    assert_eq!(table.columns[4].column_type, "date");
    assert_eq!(table.columns[0].format, "name");

    assert_eq!(table.rows.len(), 3);
    let large = &table.rows[0];
    assert_eq!(
        large.cells,
        [
            json!("large"),
            json!("large"),
            json!(10),
            json!(null),
            json!("15h"),
            json!("team-a")
        ]
    );
    let meta = large.meta.as_ref().expect("metadata row identity");
    assert_eq!(&*meta.name, "large");
    assert_eq!(meta.namespace.as_deref(), Some(NS));
    assert!(meta.uid.is_some() && meta.resource_version.is_some() && meta.creation.is_some());
    assert_eq!(large.object, None, "Metadata rows hold no object copy");
    assert!(table.resource_version.is_some());
    assert_eq!(table.continue_token, None);

    let request = &api
        .requests()
        .into_iter()
        .find(|r| r.path == WIDGETS)
        .unwrap();
    assert_eq!(request.accept.as_deref(), Some(TABLE_ACCEPT));
    assert_eq!(
        TABLE_ACCEPT,
        "application/json;as=Table;v=v1;g=meta.k8s.io,application/json"
    );
    assert_eq!(query_of(&api, WIDGETS, false, 0), "includeObject=Metadata");
}

#[tokio::test]
async fn pods_have_the_wide_columns_of_kubectl_get_pods_o_wide() {
    let api = server();
    api.reply(PODS, 200, pods_table());
    let table = adapter(&api, TableConfig::default())
        .list_table(&pod_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap();
    assert_eq!(
        names(&table.columns),
        [
            "Name",
            "Ready",
            "Status",
            "Restarts",
            "Age",
            "IP",
            "Node",
            "Nominated Node",
            "Readiness Gates"
        ]
    );
    let default: Vec<_> = table
        .columns
        .iter()
        .filter(|c| c.is_default())
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(default, ["Name", "Ready", "Status", "Restarts", "Age"]);
    assert_eq!(table.rows.len(), 2);
    assert!(table.rows.iter().all(|r| r.cells.len() == 9));
    assert_eq!(table.rows[0].cells[2], json!("Running"));
}

#[tokio::test]
async fn a_server_that_ignores_the_accept_header_falls_back_to_objects() {
    let api = server();
    api.reply(WIDGETS, 200, widgets_plain());
    let table = adapter(&api, TableConfig::default())
        .list_table(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap();
    assert_eq!(table.source, TableSource::Objects);
    assert!(!table.source.is_server());
    assert_eq!(names(&table.columns), ["Name", "Created At"]);
    assert_eq!(table.columns[1].column_type, "date");
    assert_eq!(table.rows.len(), 3);
    assert_eq!(
        table.rows[0].cells,
        [json!("large"), json!("2026-10-03T23:25:40Z")]
    );
    assert_eq!(table.rows[0].meta.as_ref().map(|m| &*m.name), Some("large"));
    assert_eq!(table.rows[0].object, None);
}

#[tokio::test]
async fn include_object_controls_row_identity_and_embedded_objects() {
    let api = server();
    let mut whole = widgets_table();
    for row in whole["rows"].as_array_mut().unwrap() {
        row["object"] = json!({"apiVersion": "test.oxikube.dev/v1", "kind": "Widget",
            "metadata": row["object"]["metadata"].clone(), "spec": {"size": "large"}});
    }
    api.reply(WIDGETS, 200, whole);
    let resources = adapter(&api, TableConfig::default());
    let object = resources
        .list_table(
            &widget_gvk(),
            Some(NS),
            &TableOptions::default().include_object(IncludeObject::Object),
        )
        .await
        .unwrap();
    let row = &object.rows[0];
    assert_eq!(row.object.as_ref().unwrap()["spec"]["size"], json!("large"));
    assert_eq!(row.meta.as_ref().map(|m| &*m.name), Some("large"));
    assert_eq!(query_of(&api, WIDGETS, false, 0), "includeObject=Object");

    let mut bare = widgets_table();
    for row in bare["rows"].as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("object");
    }
    api.reply(WIDGETS, 200, bare);
    let none = resources
        .list_table(
            &widget_gvk(),
            Some(NS),
            &TableOptions::default().include_object(IncludeObject::None),
        )
        .await
        .unwrap();
    assert!(
        none.rows
            .iter()
            .all(|r| r.meta.is_none() && r.object.is_none())
    );
    assert_eq!(query_of(&api, WIDGETS, false, 1), "includeObject=None");
}

#[tokio::test]
async fn paging_and_selectors_reach_the_server() {
    let api = server();
    api.reply(
        WIDGETS,
        200,
        widget_list(&[("a", "1", 1)], "9", Some("next")),
    );
    let options = TableOptions::default().list(
        ListOptions::default()
            .labels("tier=a")
            .limit(1)
            .continue_from("prev"),
    );
    let table = adapter(&api, TableConfig::default())
        .list_table(&widget_gvk(), Some(NS), &options)
        .await
        .unwrap();
    assert_eq!(table.continue_token.as_deref(), Some("next"));
    assert_eq!(table.resource_version.as_deref(), Some("9"));
    assert_eq!(
        query_of(&api, WIDGETS, false, 0),
        "labelSelector=tier=a&limit=1&continue=prev&includeObject=Metadata"
    );
}

#[tokio::test]
async fn all_namespaces_lists_the_cluster_wide_collection() {
    let api = server();
    api.reply(ALL_WIDGETS, 200, widgets_table());
    let table = adapter(&api, TableConfig::default())
        .list_table(&widget_gvk(), None, &TableOptions::default())
        .await
        .unwrap();
    assert_eq!(table.rows.len(), 3);
    assert_eq!(api.hits(ALL_WIDGETS), 1);
}

#[tokio::test]
async fn failures_map_to_port_error_kinds() {
    let api = server();
    api.reply(
        WIDGETS,
        403,
        status_body(403, "Forbidden", "widgets is forbidden"),
    );
    let resources = adapter(&api, TableConfig::default());
    let err = resources
        .list_table(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);

    let unknown = oxikube_domain::ids::Gvk::new("nope.dev", "v1", "Nope");
    let err = resources
        .list_table(&unknown, Some(NS), &TableOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);

    api.reply(PODS, 410, status_body(410, "Expired", "continue too old"));
    let err = resources
        .list_table(
            &pod_gvk(),
            Some(NS),
            &TableOptions::default().list(ListOptions::default().continue_from("old")),
        )
        .await
        .unwrap_err();
    assert!(is_list_expired(&err));

    api.reply(GADGETS, 200, json!("not a table"));
    let err = resources
        .list_table(&gadget_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
}

#[tokio::test(start_paused = true)]
async fn a_hung_server_times_out() {
    let api = server();
    api.stall(WIDGETS);
    let err = adapter(&api, TableConfig::default())
        .list_table(&widget_gvk(), Some(NS), &TableOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
    assert!(err.is_retryable());
}
