//! Fixtures: a fake API server with discovery for pods, the `Widget` CRD and a list-only
//! aggregated kind, plus recorded Table responses and watch-event builders.

use serde_json::{Value, json};

use crate::discovery::{DiscoveryConfig, KubeDiscovery};
use crate::fake_api::FakeApi;
use crate::resources::{KubeResources, ResourcesConfig};
use crate::table::TableConfig;
use oxikube_domain::ids::Gvk;

pub(super) const PODS: &str = "/api/v1/namespaces/oxikube-fixtures/pods";
pub(super) const WIDGETS: &str = "/apis/test.oxikube.dev/v1/namespaces/oxikube-fixtures/widgets";
pub(super) const ALL_WIDGETS: &str = "/apis/test.oxikube.dev/v1/widgets";
pub(super) const GADGETS: &str = "/apis/agg.oxikube.dev/v1/namespaces/oxikube-fixtures/gadgets";
pub(super) const NS: &str = "oxikube-fixtures";

const WIDGETS_TABLE: &str = include_str!("../../../tests/fixtures/table/widgets.json");
const PODS_TABLE: &str = include_str!("../../../tests/fixtures/table/pods.json");
const WIDGETS_PLAIN: &str = include_str!("../../../tests/fixtures/table/widgets-plain-list.json");

pub(super) fn widgets_table() -> Value {
    serde_json::from_str(WIDGETS_TABLE).expect("widgets fixture")
}

pub(super) fn pods_table() -> Value {
    serde_json::from_str(PODS_TABLE).expect("pods fixture")
}

pub(super) fn widgets_plain() -> Value {
    serde_json::from_str(WIDGETS_PLAIN).expect("plain list fixture")
}

pub(super) fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

pub(super) fn widget_gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

/// An aggregated-API kind served with `list` but not `watch`.
pub(super) fn gadget_gvk() -> Gvk {
    Gvk::new("agg.oxikube.dev", "v1", "Gadget")
}

fn resource(name: &str, kind: &str, verbs: &[&str]) -> Value {
    json!({"name": name, "singularName": "", "namespaced": true, "kind": kind, "verbs": verbs})
}

fn group(name: &str) -> Value {
    let gv = json!({"groupVersion": format!("{name}/v1"), "version": "v1"});
    json!({"name": name, "versions": [gv], "preferredVersion": gv})
}

/// A server with discovery scripted and nothing else.
pub(super) fn server() -> FakeApi {
    let api = FakeApi::new();
    let full = ["get", "list", "watch"];
    api.reply(
        "/api",
        200,
        json!({"kind": "APIVersions", "versions": ["v1"]}),
    );
    api.reply(
        "/apis",
        200,
        json!({"kind": "APIGroupList", "groups": [group("test.oxikube.dev"), group("agg.oxikube.dev")]}),
    );
    api.reply(
        "/api/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "v1", "resources": [resource("pods", "Pod", &full)]}),
    );
    api.reply(
        "/apis/test.oxikube.dev/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "test.oxikube.dev/v1", "resources": [resource("widgets", "Widget", &full)]}),
    );
    api.reply(
        "/apis/agg.oxikube.dev/v1",
        200,
        json!({"kind": "APIResourceList", "groupVersion": "agg.oxikube.dev/v1", "resources": [resource("gadgets", "Gadget", &["get", "list"])]}),
    );
    api
}

/// `KubeResources` over `api` with `table` settings.
pub(super) fn adapter(api: &FakeApi, table: TableConfig) -> KubeResources {
    let discovery = KubeDiscovery::with_config(
        api.client(),
        DiscoveryConfig {
            aggregated: false,
            ..DiscoveryConfig::default()
        },
    );
    let config = ResourcesConfig {
        table,
        ..ResourcesConfig::default()
    };
    KubeResources::with_config(api.client(), discovery, config)
}

/// A one-row widget Table as the server sends it in a watch event: `columnDefinitions` only
/// on the first event of a connection, `null` on the others (as recorded from kind).
pub(super) fn widget_event_table(name: &str, rv: &str, replicas: i64, columns: bool) -> Value {
    let full = widgets_table();
    let definitions = if columns {
        full["columnDefinitions"].clone()
    } else {
        Value::Null
    };
    json!({
        "kind": "Table", "apiVersion": "meta.k8s.io/v1",
        "metadata": {"resourceVersion": rv},
        "columnDefinitions": definitions,
        "rows": [widget_row(name, rv, replicas)],
    })
}

/// One widget row with `includeObject=Metadata`.
pub(super) fn widget_row(name: &str, rv: &str, replicas: i64) -> Value {
    json!({
        "cells": [name, "large", replicas, null, "1m", "team-a"],
        "object": {
            "kind": "PartialObjectMetadata", "apiVersion": "meta.k8s.io/v1",
            "metadata": {
                "name": name, "namespace": NS, "uid": format!("uid-{name}"),
                "resourceVersion": rv, "creationTimestamp": "2026-10-03T23:25:40Z",
            },
        },
    })
}

/// A widget Table list page holding `rows` (name, resourceVersion, replicas).
pub(super) fn widget_list(
    rows: &[(&str, &str, i64)],
    rv: &str,
    continue_token: Option<&str>,
) -> Value {
    let mut table = widgets_table();
    table["rows"] = rows
        .iter()
        .map(|(name, row_rv, replicas)| widget_row(name, row_rv, *replicas))
        .collect();
    table["metadata"] = json!({"resourceVersion": rv});
    if let Some(token) = continue_token {
        table["metadata"]["continue"] = json!(token);
    }
    table
}

/// A watch event line.
pub(super) fn event(kind: &str, object: Value) -> Value {
    json!({"type": kind, "object": object})
}

/// A Table bookmark event at `rv`.
pub(super) fn bookmark(rv: &str) -> Value {
    event(
        "BOOKMARK",
        json!({"kind": "Table", "apiVersion": "meta.k8s.io/v1", "metadata": {"resourceVersion": rv},
               "columnDefinitions": null, "rows": [{"cells": ["", null, null, null, null, null], "object": null}]}),
    )
}

/// A watch `ERROR` event carrying a `Status` with `code`.
pub(super) fn error_event(code: u16, reason: &str) -> Value {
    event(
        "ERROR",
        crate::fake_api::status_body(code, reason, "watch failed"),
    )
}

/// The decoded query of the `n`th non-watch / watch request to `path`.
pub(super) fn query_of(api: &FakeApi, path: &str, watch: bool, n: usize) -> String {
    let raw = api
        .requests()
        .into_iter()
        .filter(|r| r.path == path && r.is_watch() == watch)
        .nth(n)
        .unwrap_or_else(|| panic!("no request #{n} (watch: {watch}) to {path}"))
        .query;
    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// How many list (not watch) requests went to `path`.
pub(super) fn lists(api: &FakeApi, path: &str) -> usize {
    api.requests()
        .iter()
        .filter(|r| r.path == path && !r.is_watch())
        .count()
}

/// How many watch requests went to `path`.
pub(super) fn watches(api: &FakeApi, path: &str) -> usize {
    api.requests()
        .iter()
        .filter(|r| r.path == path && r.is_watch())
        .count()
}
