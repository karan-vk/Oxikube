//! Unit tests for the OpenAPI v3 adapter: lazy fetches, single-flight,
//! disk-cache reuse and invalidation, against [`FakeApi`](crate::fake_api::FakeApi).

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_ports::{FsPort, SchemaPort};
use oxikube_testkit::FakeFsPort;
use serde_json::json;

use super::service::{OpenApiConfig, OpenApiSchemas};
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
        files[0].path.ends_with("apis_apps_v1-BBB_.json"),
        "{:?}",
        files[0].path
    );
}
