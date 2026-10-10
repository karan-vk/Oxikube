//! Unit tests for the OpenAPI v3 adapter: lazy fetches, single-flight,
//! disk-cache reuse and invalidation, against [`FakeApi`](crate::fake_api::FakeApi).

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_ports::{FsPort, SchemaPort};
use oxikube_testkit::FakeFsPort;
use serde_json::json;

use super::index::Index;
use super::{OpenApiConfig, OpenApiSchemas};
use crate::fake_api::{FakeApi, version_body};

fn cluster() -> ClusterId {
    ClusterId::new("openapi-test", &ContextName::new("kind-oxikube"))
}

fn stateful_set() -> Gvk {
    Gvk::new("apps", "v1", "StatefulSet")
}

fn deployment() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

fn index_body() -> serde_json::Value {
    json!({
        "paths": {
            "api/v1": {"serverRelativeURL": "/openapi/v3/api/v1?hash=AAA="},
            "apis/apps/v1": {"serverRelativeURL": "/openapi/v3/apis/apps/v1?hash=BBB="},
        }
    })
}

fn apps_document() -> serde_json::Value {
    json!({
        "components": {
            "schemas": {
                "io.k8s.api.apps.v1.Deployment": {
                    "description": "Deployment enables declarative updates for Pods.",
                    "properties": {
                        "spec": {"$ref": "#/components/schemas/io.k8s.api.apps.v1.DeploymentSpec"}
                    },
                    "required": ["spec"],
                    "type": "object",
                    "x-kubernetes-group-version-kind": [
                        {"group": "apps", "kind": "Deployment", "version": "v1"}
                    ]
                },
                "io.k8s.api.apps.v1.StatefulSet": {
                    "properties": {"spec": {"type": "object"}},
                    "type": "object",
                    "x-kubernetes-group-version-kind": [
                        {"group": "apps", "kind": "StatefulSet", "version": "v1"}
                    ]
                },
                "io.k8s.api.apps.v1.DeploymentSpec": {
                    "properties": {
                        "replicas": {"format": "int32", "type": "integer"}
                    },
                    "type": "object"
                }
            }
        }
    })
}

/// A fake server with the index, `/version` and the apps/v1 document scripted.
fn server() -> FakeApi {
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/version", 200, version_body("v1.31.0"));
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    api
}

fn service(api: &FakeApi, fs: &Arc<FakeFsPort>) -> OpenApiSchemas {
    OpenApiSchemas::new(api.client(), cluster())
        .with_disk_cache(fs.clone(), PathBuf::from("/cache"))
}

fn memory_only(api: &FakeApi) -> OpenApiSchemas {
    OpenApiSchemas::new(api.client(), cluster())
}

#[tokio::test]
async fn second_lookup_in_a_group_makes_no_request() {
    let api = server();
    let svc = service(&api, &Arc::new(FakeFsPort::new()));

    let schema = svc
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    let replicas = schema
        .properties
        .get("spec")
        .and_then(|spec| spec.properties.get("replicas"))
        .expect("spec.replicas");
    assert_eq!(replicas.format.as_deref(), Some("int32"));

    // The same GVK again is a memory hit that shares the Arc.
    let again = svc
        .schema_for(&cluster(), &deployment())
        .await
        .expect("cached");
    assert!(Arc::ptr_eq(&schema, &again), "memory cache shares the Arc");
    // Another kind of the same group reads the document from the disk cache.
    svc.schema_for(&cluster(), &stateful_set())
        .await
        .expect("sibling kind");
    assert_eq!(api.hits("/openapi/v3"), 1);
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 1);
    assert_eq!(
        api.hits("/openapi/v3/api/v1"),
        0,
        "other groups stay unfetched"
    );
}

#[tokio::test]
async fn concurrent_callers_share_one_fetch() {
    let api = server();
    let svc = Arc::new(memory_only(&api));
    let calls = (0..4).map(|_| {
        let svc = svc.clone();
        tokio::spawn(async move { svc.schema_for(&cluster(), &deployment()).await })
    });
    for result in futures::future::join_all(calls).await {
        assert!(result.expect("task").is_ok());
    }
    assert_eq!(api.hits("/openapi/v3"), 1, "one index fetch");
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 1, "single-flight");
}

#[tokio::test]
async fn disk_cache_is_keyed_by_server_version_and_index_hash() {
    let fs = Arc::new(FakeFsPort::new());
    service(&server(), &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("first fetch");

    // A fresh instance over the same filesystem re-reads the index but not the group.
    let same = server();
    service(&same, &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("disk hit");
    assert_eq!(same.hits("/openapi/v3/apis/apps/v1"), 0);

    // A new index hash refetches the group document.
    let rehashed = FakeApi::new();
    rehashed.reply("/version", 200, version_body("v1.31.0"));
    rehashed.reply(
        "/openapi/v3",
        200,
        json!({"paths": {
            "apis/apps/v1": {"serverRelativeURL": "/openapi/v3/apis/apps/v1?hash=CCC="},
        }}),
    );
    rehashed.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    service(&rehashed, &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("hash change refetches");
    assert_eq!(rehashed.hits("/openapi/v3/apis/apps/v1"), 1);

    // A server upgrade (same hash, new version) refetches too and drops the old files.
    let upgraded = FakeApi::new();
    upgraded.reply("/openapi/v3", 200, index_body());
    upgraded.reply("/version", 200, version_body("v1.32.0"));
    upgraded.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    service(&upgraded, &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("upgrade refetches");
    assert_eq!(upgraded.hits("/openapi/v3/apis/apps/v1"), 1);
    let root = PathBuf::from("/cache").join(cluster().as_str());
    assert!(
        fs.list(&root.join("v1.31.0"))
            .await
            .map_or(true, |files| files.is_empty()),
        "the old version's files are gone"
    );
    assert_eq!(
        fs.list(&root.join("v1.32.0")).await.expect("new dir").len(),
        1
    );
}

#[tokio::test]
async fn invalidate_rereads_the_index_and_refetches_only_changed_documents() {
    let api = server();
    // After the invalidate the index lists a new hash for apps/v1.
    api.reply(
        "/openapi/v3",
        200,
        json!({"paths": {
            "apis/apps/v1": {"serverRelativeURL": "/openapi/v3/apis/apps/v1?hash=NEW="},
        }}),
    );
    let svc = service(&api, &Arc::new(FakeFsPort::new()));
    let before = svc
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    svc.invalidate(&cluster()).await.expect("invalidate");
    let after = svc
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    assert!(!Arc::ptr_eq(&before, &after), "memory was dropped");
    assert_eq!(api.hits("/openapi/v3"), 2);
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 2, "the hash changed");

    // An unchanged hash is served from disk after an invalidate.
    let quiet = server();
    let svc = service(&quiet, &Arc::new(FakeFsPort::new()));
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    svc.invalidate(&cluster()).await.expect("invalidate");
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    assert_eq!(quiet.hits("/openapi/v3"), 2, "the index is re-read");
    assert_eq!(
        quiet.hits("/openapi/v3/apis/apps/v1"),
        1,
        "the document is not"
    );
}

#[tokio::test]
async fn a_miss_rereads_an_old_index_and_finds_a_kind_added_since() {
    let api = FakeApi::new();
    api.reply("/version", 200, version_body("v1.31.0"));
    // First the index knows only core; later it also lists apps/v1.
    api.reply(
        "/openapi/v3",
        200,
        json!({"paths": {"api/v1": {"serverRelativeURL": "/openapi/v3/api/v1?hash=AAA="}}}),
    );
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    let config = OpenApiConfig {
        refresh_on_miss_after: std::time::Duration::ZERO,
        ..OpenApiConfig::default()
    };
    let svc = OpenApiSchemas::with_config(api.client(), cluster(), config);
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("found after the index refresh");
    assert_eq!(api.hits("/openapi/v3"), 2);

    // A young index answers NotFound without another round trip.
    let young = OpenApiSchemas::new(api.client(), cluster());
    let err = young
        .schema_for(&cluster(), &Gvk::new("batch", "v1", "Job"))
        .await
        .expect_err("no such document");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let hits = api.hits("/openapi/v3");
    young
        .schema_for(&cluster(), &Gvk::new("batch", "v1", "Job"))
        .await
        .expect_err("still none");
    assert_eq!(
        api.hits("/openapi/v3"),
        hits,
        "no refetch inside the window"
    );
}

#[tokio::test]
async fn unknown_kind_in_a_known_group_is_not_found() {
    let api = server();
    let svc = memory_only(&api);
    let err = svc
        .schema_for(&cluster(), &Gvk::new("apps", "v1", "ReplicaSet"))
        .await
        .expect_err("no such schema");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn missing_v3_endpoint_is_unsupported() {
    let api = FakeApi::new();
    let err = memory_only(&api)
        .schema_for(&cluster(), &deployment())
        .await
        .expect_err("no endpoint");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn another_cluster_is_rejected() {
    let svc = memory_only(&server());
    let other = ClusterId::new("other", &ContextName::new("elsewhere"));
    let err = svc
        .schema_for(&other, &deployment())
        .await
        .expect_err("wrong cluster");
    assert_eq!(err.kind(), ErrorKind::Validation);
    assert_eq!(
        svc.invalidate(&other).await.unwrap_err().kind(),
        ErrorKind::Validation
    );
}

#[tokio::test]
async fn a_corrupt_disk_entry_is_refetched() {
    let fs = Arc::new(FakeFsPort::new());
    let api = server();
    service(&api, &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("first fetch");
    let dir = PathBuf::from("/cache")
        .join(cluster().as_str())
        .join("v1.31.0");
    let file = fs.list(&dir).await.expect("list").remove(0).path;
    fs.write(&file, b"not json").await.expect("corrupt");

    let again = server();
    service(&again, &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("falls back to the server");
    assert_eq!(again.hits("/openapi/v3/apis/apps/v1"), 1);
}

#[tokio::test]
async fn disk_cache_holds_one_public_document_per_group_version() {
    let fs = Arc::new(FakeFsPort::new());
    service(&server(), &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    let dir = PathBuf::from("/cache")
        .join(cluster().as_str())
        .join("v1.31.0");
    let files = fs.list(&dir).await.expect("list");
    assert_eq!(files.len(), 1);
    assert!(
        files[0].path.ends_with("apis_2fapps_2fv1-BBB_3d.json"),
        "{:?}",
        files[0].path
    );
}

#[tokio::test]
async fn the_disk_file_is_exactly_the_public_group_document() {
    let fs = Arc::new(FakeFsPort::new());
    service(&server(), &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    let dir = PathBuf::from("/cache")
        .join(cluster().as_str())
        .join("v1.31.0");
    let file = fs.list(&dir).await.expect("list").remove(0).path;
    let stored: serde_json::Value =
        serde_json::from_slice(&fs.read(&file).await.expect("read")).expect("json");
    assert_eq!(
        stored,
        apps_document(),
        "schemas only: no wrapper, no object data"
    );
}

#[tokio::test]
async fn an_index_without_hashes_is_never_cached_on_disk() {
    let fs = Arc::new(FakeFsPort::new());
    let api = FakeApi::new();
    api.reply(
        "/openapi/v3",
        200,
        json!({"paths": {"apis/apps/v1": {"serverRelativeURL": "/openapi/v3/apis/apps/v1"}}}),
    );
    api.reply("/version", 200, version_body("v1.31.0"));
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    service(&api, &fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    let root = PathBuf::from("/cache");
    assert!(
        fs.list(&root).await.map_or(true, |files| files.is_empty()),
        "nothing validates a hashless file, so nothing is written"
    );
}

#[tokio::test]
async fn kinds_of_one_group_download_it_once_even_without_a_disk_cache() {
    let api = server();
    let svc = memory_only(&api);
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("deployment");
    svc.schema_for(&cluster(), &stateful_set())
        .await
        .expect("stateful set");
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 1);

    // An invalidate drops the held document with the rest of the memory state.
    svc.invalidate(&cluster()).await.expect("invalidate");
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("deployment");
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 2);
}

#[tokio::test]
async fn a_server_without_openapi_v3_is_asked_once_until_invalidated() {
    let api = FakeApi::new();
    let svc = memory_only(&api);
    for _ in 0..3 {
        let err = svc
            .schema_for(&cluster(), &deployment())
            .await
            .expect_err("unsupported");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    }
    assert_eq!(api.hits("/openapi/v3"), 1);
    svc.invalidate(&cluster()).await.expect("invalidate");
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect_err("still unsupported");
    assert_eq!(
        api.hits("/openapi/v3"),
        2,
        "asked again after an invalidate"
    );
}

#[tokio::test]
async fn a_lookup_started_before_an_invalidate_does_not_cache_its_result() {
    let api = server();
    let svc = Arc::new(memory_only(&api));
    // The invalidate lands while the lookup is awaiting its fetches.
    let lookup = tokio::spawn({
        let svc = svc.clone();
        async move { svc.schema_for(&cluster(), &deployment()).await }
    });
    tokio::task::yield_now().await;
    svc.invalidate(&cluster()).await.expect("invalidate");
    lookup
        .await
        .expect("task")
        .expect("the caller still gets its schema");
    // Nothing from the pre-invalidate lookup was kept: the next one fetches again.
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 2);
}

fn example_index(hash: &str) -> serde_json::Value {
    json!({"paths": {"apis/example.dev/v1": {
        "serverRelativeURL": format!("/openapi/v3/apis/example.dev/v1?hash={hash}")
    }}})
}

fn example_document(kind: &str) -> serde_json::Value {
    json!({"components": {"schemas": {format!("dev.example.v1.{kind}"): {
        "type": "object",
        "properties": {"spec": {"type": "object"}},
        "x-kubernetes-group-version-kind": [{"group": "example.dev", "kind": kind, "version": "v1"}]
    }}}})
}

#[tokio::test]
async fn a_miss_from_a_lookup_that_an_invalidate_overtook_is_not_remembered() {
    let gizmo = Gvk::new("example.dev", "v1", "Gizmo");
    // Replies are served in order: the lookup below reads the index and document as they were
    // before the CRD was applied, the next lookup the current ones.
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, example_index("OLD="));
    api.reply(
        "/openapi/v3/apis/example.dev/v1",
        200,
        example_document("Sprocket"),
    );
    api.reply("/openapi/v3", 200, example_index("NEW="));
    api.reply(
        "/openapi/v3/apis/example.dev/v1",
        200,
        example_document("Gizmo"),
    );
    let svc = Arc::new(memory_only(&api));

    // The lookup holds the old index and waits for the group's document when the invalidate
    // lands, so its NotFound is about documents the invalidate discarded.
    let group = Index::key_for("example.dev", "v1");
    let flight = Arc::new(tokio::sync::Mutex::new(()));
    let parked = flight.clone().lock_owned().await;
    svc.groups.lock().insert(group, flight);
    let lookup = tokio::spawn({
        let svc = svc.clone();
        let gizmo = gizmo.clone();
        async move { svc.schema_for(&cluster(), &gizmo).await }
    });
    while api.hits("/openapi/v3") == 0 {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    svc.invalidate(&cluster()).await.expect("invalidate");
    drop(parked);
    let err = lookup
        .await
        .expect("task")
        .expect_err("the old document has no Gizmo");
    assert_eq!(err.kind(), ErrorKind::NotFound);

    // The re-read the invalidate asked for happens: the stale miss is not answered from memory.
    svc.schema_for(&cluster(), &gizmo)
        .await
        .expect("the new kind is found at once");
    assert_eq!(api.hits("/openapi/v3"), 2);
}

#[tokio::test]
async fn unsupported_and_misses_are_retried_once_their_window_has_passed() {
    let config = OpenApiConfig {
        refresh_on_miss_after: std::time::Duration::ZERO,
        ..OpenApiConfig::default()
    };
    let api = FakeApi::new();
    let svc = OpenApiSchemas::with_config(api.client(), cluster(), config);
    for _ in 0..2 {
        let err = svc
            .schema_for(&cluster(), &deployment())
            .await
            .expect_err("unsupported");
        assert_eq!(err.kind(), ErrorKind::Unsupported);
    }
    assert_eq!(api.hits("/openapi/v3"), 2, "a blip does not stick");
}

fn gizmo_document(extra_field: Option<&str>) -> serde_json::Value {
    let mut spec = json!({"type": "object", "properties": {}});
    if let Some(field) = extra_field {
        spec["properties"][field] = json!({"type": "string"});
    }
    json!({"components": {"schemas": {"dev.example.v1.Gizmo": {
        "type": "object",
        "properties": {"spec": spec},
        "x-kubernetes-group-version-kind": [{"group": "example.dev", "kind": "Gizmo", "version": "v1"}]
    }}}})
}

fn has_color(schema: &oxikube_domain::schema::JsonSchema) -> bool {
    schema
        .properties
        .get("spec")
        .and_then(|spec| spec.properties.get("color"))
        .is_some()
}

#[tokio::test]
async fn a_schema_cached_from_an_index_that_lagged_the_invalidate_is_replaced() {
    let gizmo = Gvk::new("example.dev", "v1", "Gizmo");
    let group = "/openapi/v3/apis/example.dev/v1";
    // Index reads in order: the first lookup; the one right after the invalidate, which still
    // lists the old hash (the server's document lags the CRD edit); the re-check, which lists
    // the new one.
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, example_index("OLD="));
    api.reply("/openapi/v3", 200, example_index("OLD="));
    api.reply("/openapi/v3", 200, example_index("NEW="));
    api.reply(group, 200, gizmo_document(None));
    api.reply(group, 200, gizmo_document(None));
    api.reply(group, 200, gizmo_document(Some("color")));
    let config = OpenApiConfig {
        settle_after_invalidate: std::time::Duration::from_millis(200),
        recheck_every: std::time::Duration::ZERO,
        ..OpenApiConfig::default()
    };
    let svc = OpenApiSchemas::with_config(api.client(), cluster(), config);

    let first = svc.schema_for(&cluster(), &gizmo).await.expect("schema");
    assert!(!has_color(&first));
    svc.invalidate(&cluster()).await.expect("invalidate");
    let stale = svc.schema_for(&cluster(), &gizmo).await.expect("schema");
    assert!(!has_color(&stale), "the index still lagged");

    // The next lookup is inside the settling time: it re-reads the index and finds the change.
    let fresh = svc.schema_for(&cluster(), &gizmo).await.expect("schema");
    assert!(has_color(&fresh), "the stale hit was replaced");
    assert_eq!(api.hits("/openapi/v3"), 3);

    // Once an index read is older than the settling time nothing is re-read any more.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    svc.schema_for(&cluster(), &gizmo).await.expect("hit");
    assert_eq!(api.hits("/openapi/v3"), 4, "one last comparison");
    svc.schema_for(&cluster(), &gizmo).await.expect("hit");
    svc.schema_for(&cluster(), &gizmo).await.expect("hit");
    assert_eq!(api.hits("/openapi/v3"), 4, "settled: memory hits only");
}

#[tokio::test]
async fn nothing_is_rechecked_without_an_invalidate_or_inside_the_interval() {
    let api = server();
    let svc = OpenApiSchemas::with_config(
        api.client(),
        cluster(),
        OpenApiConfig {
            recheck_every: std::time::Duration::ZERO,
            ..OpenApiConfig::default()
        },
    );
    for _ in 0..3 {
        svc.schema_for(&cluster(), &deployment())
            .await
            .expect("schema");
    }
    assert_eq!(api.hits("/openapi/v3"), 1, "never invalidated: no re-check");

    // Settling, but the index is younger than the interval: no request either.
    let slow = OpenApiSchemas::new(api.client(), cluster());
    slow.schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    slow.invalidate(&cluster()).await.expect("invalidate");
    for _ in 0..3 {
        slow.schema_for(&cluster(), &deployment())
            .await
            .expect("schema");
    }
    assert_eq!(api.hits("/openapi/v3"), 3, "one load after the invalidate");
}
