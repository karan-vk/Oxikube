//! Unit tests for the OpenAPI v3 adapter: lazy fetches, single-flight,
//! disk-cache reuse and invalidation, against [`FakeApi`](crate::fake_api::FakeApi).

use std::path::PathBuf;
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_ports::{FsPort, SchemaPort};
use oxikube_testkit::FakeFsPort;
use serde_json::json;

use super::service::OpenApiSchemas;
use crate::fake_api::FakeApi;

fn cluster() -> ClusterId {
    ClusterId::new("openapi-test", &ContextName::new("kind-oxikube"))
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

fn service(api: &FakeApi, fs: Arc<FakeFsPort>) -> OpenApiSchemas {
    OpenApiSchemas::new(api.client(), fs, PathBuf::from("/cache"))
}

#[tokio::test]
async fn second_schema_in_a_group_makes_no_request() {
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    let svc = service(&api, Arc::new(FakeFsPort::new()));

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

    // Same GVK again, then the group is cached: no further requests at all.
    let again = svc
        .schema_for(&cluster(), &deployment())
        .await
        .expect("cached");
    assert!(Arc::ptr_eq(&schema, &again), "memory cache shares the Arc");
    assert_eq!(api.hits("/openapi/v3"), 1);
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 1);
}

#[tokio::test]
async fn concurrent_callers_share_one_fetch() {
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    let svc = Arc::new(service(&api, Arc::new(FakeFsPort::new())));

    let first = tokio::spawn({
        let svc = svc.clone();
        let cluster = cluster();
        async move { svc.schema_for(&cluster, &deployment()).await }
    });
    let second = tokio::spawn({
        let svc = svc.clone();
        let cluster = cluster();
        async move { svc.schema_for(&cluster, &deployment()).await }
    });
    let (first, second) = tokio::join!(first, second);
    assert!(first.expect("task").is_ok());
    assert!(second.expect("task").is_ok());
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 1, "single-flight");
}

#[tokio::test]
async fn disk_cache_survives_a_new_instance_until_the_hash_changes() {
    let fs = Arc::new(FakeFsPort::new());
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    service(&api, fs.clone())
        .schema_for(&cluster(), &deployment())
        .await
        .expect("first fetch");

    // A fresh instance over the same filesystem re-reads the index but not the group.
    let api2 = FakeApi::new();
    api2.reply("/openapi/v3", 200, index_body());
    service(&api2, fs.clone())
        .schema_for(&cluster(), &deployment())
        .await
        .expect("disk hit");
    assert_eq!(api2.hits("/openapi/v3/apis/apps/v1"), 0);

    // A new index hash refetches the group document.
    let api3 = FakeApi::new();
    api3.reply(
        "/openapi/v3",
        200,
        json!({"paths": {
            "apis/apps/v1": {"serverRelativeURL": "/openapi/v3/apis/apps/v1?hash=CCC="},
        }}),
    );
    api3.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    service(&api3, fs)
        .schema_for(&cluster(), &deployment())
        .await
        .expect("hash change refetches");
    assert_eq!(api3.hits("/openapi/v3/apis/apps/v1"), 1);
}

#[tokio::test]
async fn invalidate_refetches_index_and_group() {
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    let svc = service(&api, Arc::new(FakeFsPort::new()));
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    svc.invalidate(&cluster()).await.expect("invalidate");
    svc.schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    assert_eq!(api.hits("/openapi/v3"), 2);
    assert_eq!(api.hits("/openapi/v3/apis/apps/v1"), 2);
}

#[tokio::test]
async fn unknown_gvk_is_not_found() {
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    let svc = service(&api, Arc::new(FakeFsPort::new()));
    let err = svc
        .schema_for(&cluster(), &Gvk::new("apps", "v1", "StatefulSet"))
        .await
        .expect_err("no such schema");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn missing_v3_endpoint_is_unsupported() {
    let api = FakeApi::new();
    let svc = service(&api, Arc::new(FakeFsPort::new()));
    let err = svc
        .schema_for(&cluster(), &deployment())
        .await
        .expect_err("no endpoint");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn disk_cache_files_hold_no_object_data() {
    let fs = Arc::new(FakeFsPort::new());
    let api = FakeApi::new();
    api.reply("/openapi/v3", 200, index_body());
    api.reply("/openapi/v3/apis/apps/v1", 200, apps_document());
    service(&api, fs.clone())
        .schema_for(&cluster(), &deployment())
        .await
        .expect("schema");
    let files = fs
        .list(&PathBuf::from("/cache").join(cluster().as_str()))
        .await
        .expect("list");
    assert_eq!(files.len(), 1, "one file per group-version");
}
