//! Kind integration for E10-S01: `SchemaPort` against the real `/openapi/v3` of the
//! kind cluster: a built-in kind (Deployment), the sample CRD (`Widget`) and a CRD created
//! while the adapter is running (found again after `invalidate`, as the session does on
//! `KindsChanged`). Needs `cargo xtask kind-up` (installs the `widgets.test.oxikube.dev` CRD)
//! and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
//!
//! The first test also prints the numbers the story's PR reports (run with `--nocapture`):
//! time to the first schema, flatten time and the retained heap of one flattened schema.
#![cfg(feature = "integration")]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::{JsonSchema, SchemaType, root_schema_for};
use oxikube_kube::{OpenApiConfig, OpenApiSchemas};
use oxikube_ports::SchemaPort;
use oxikube_testkit::FakeFsPort;

use common::{DEADLINE, TestCrd};

/// Counts live heap bytes of the whole test process, to size one flattened schema.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: forwards every call to the system allocator unchanged and only updates a counter.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(
            new_size as isize - layout.size() as isize,
            Ordering::Relaxed,
        );
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn cluster(kind: &common::Kind) -> ClusterId {
    ClusterId::new("kind-test", &kind.context)
}

fn config() -> OpenApiConfig {
    OpenApiConfig {
        request_timeout: Duration::from_secs(30),
        refresh_on_miss_after: Duration::ZERO,
        ..OpenApiConfig::default()
    }
}

fn node_count(schema: &JsonSchema) -> usize {
    1 + schema.properties.values().map(node_count).sum::<usize>()
        + schema.items.as_deref().map_or(0, node_count)
}

fn truncated_nodes(schema: &JsonSchema) -> usize {
    usize::from(schema.truncated)
        + schema
            .properties
            .values()
            .map(truncated_nodes)
            .sum::<usize>()
        + schema.items.as_deref().map_or(0, truncated_nodes)
}

#[tokio::test]
async fn deployment_and_widget_schemas_come_from_the_server() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let id = cluster(&kind);
    let fs = Arc::new(FakeFsPort::new());
    let svc = OpenApiSchemas::with_config((*client).clone(), id.clone(), config())
        .with_disk_cache(fs.clone(), PathBuf::from("/openapi-kind-cache"));

    let started = Instant::now();
    let deployment = svc
        .schema_for(&id, &Gvk::new("apps", "v1", "Deployment"))
        .await
        .expect("deployment schema");
    let first = started.elapsed();
    let replicas = deployment
        .properties
        .get("spec")
        .and_then(|spec| spec.properties.get("replicas"))
        .expect("spec.replicas");
    assert_eq!(
        replicas.types,
        vec![SchemaType::Integer],
        "replicas is an integer"
    );
    assert_eq!(replicas.format.as_deref(), Some("int32"));
    assert!(deployment.required.contains(&"spec".to_owned()) || deployment.has_property("spec"));

    assert_eq!(
        truncated_nodes(&deployment),
        0,
        "a built-in kind flattens completely (no cycle, no depth cut)"
    );
    let pod = svc
        .schema_for(&id, &Gvk::new("", "v1", "Pod"))
        .await
        .expect("pod schema");
    assert_eq!(truncated_nodes(&pod), 0);
    assert!(pod.has_property("spec") && pod.has_property("metadata"));

    let widget = svc
        .schema_for(&id, &Gvk::new("test.oxikube.dev", "v1", "Widget"))
        .await
        .expect("widget schema");
    assert!(
        widget.has_property("spec"),
        "the sample CRD resolves by group-version-kind"
    );

    let again = svc
        .schema_for(&id, &Gvk::new("apps", "v1", "Deployment"))
        .await
        .expect("cached deployment schema");
    assert!(Arc::ptr_eq(&deployment, &again));

    // A second adapter over the same cache re-reads the index but not the document.
    let warm = OpenApiSchemas::with_config((*client).clone(), id.clone(), config())
        .with_disk_cache(fs, PathBuf::from("/openapi-kind-cache"));
    let started_warm = Instant::now();
    warm.schema_for(&id, &Gvk::new("apps", "v1", "Deployment"))
        .await
        .expect("warm deployment schema");
    let warm_time = started_warm.elapsed();

    // The numbers the PR reports: flatten the same document repeatedly, size the result.
    let get = |url: String| {
        let client = client.clone();
        async move {
            let request = http::Request::get(url)
                .body(Vec::<u8>::new())
                .expect("request");
            client.request_text(request).await.expect("GET")
        }
    };
    let index: serde_json::Value =
        serde_json::from_str(&get("/openapi/v3".to_owned()).await).expect("index json");
    let url = index["paths"]["apis/apps/v1"]["serverRelativeURL"]
        .as_str()
        .expect("apps/v1 in the index")
        .to_owned();
    let text = get(url).await;
    let document: serde_json::Value = serde_json::from_str(&text).expect("document json");
    let gvk = Gvk::new("apps", "v1", "Deployment");
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let started = Instant::now();
        let flat = root_schema_for(&document, &gvk).expect("root");
        best = best.min(started.elapsed());
        drop(flat);
    }
    let before = LIVE.load(Ordering::Relaxed);
    let flat = root_schema_for(&document, &gvk).expect("root");
    let retained = LIVE.load(Ordering::Relaxed) - before;
    eprintln!(
        "E10-S01 perf: first schema (index + version + apps/v1 fetch + parse + flatten) {first:?}; \
         warm disk-cache start {warm_time:?}; apps/v1 document {} KiB; flatten best of 5 {best:?}; \
         Deployment schema {} nodes, retained {} KiB; Pod schema {} nodes",
        text.len() / 1024,
        node_count(&flat),
        retained / 1024,
        node_count(&pod),
    );
    assert!(first < DEADLINE * 4, "first schemas arrive promptly");
}

#[tokio::test]
async fn a_crd_created_while_running_is_found_after_invalidate() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let id = cluster(&kind);
    let svc = OpenApiSchemas::with_config((*client).clone(), id.clone(), config());
    // Load the index before the CRD exists.
    svc.schema_for(&id, &Gvk::new("apps", "v1", "Deployment"))
        .await
        .expect("deployment schema");

    let crd = TestCrd::create(&client, kind.context.as_str()).await;
    // The server's OpenAPI document trails the CRD by a moment: poll, invalidating as the
    // session does on each `KindsChanged`.
    let schema = common::wait_until("the new CRD's schema", DEADLINE, || async {
        svc.invalidate(&id).await.expect("invalidate");
        svc.schema_for(&id, &crd.gvk).await.ok()
    })
    .await;
    assert!(
        schema.xk8s.preserve_unknown_fields,
        "x-kubernetes-preserve-unknown-fields marks the root open: {schema:?}"
    );

    crd.delete(&client).await;
}

/// Editing a CRD's schema in place changes no kind record, so discovery reports no
/// `KindsChanged`; `SchemasChanged` is what tells the session to invalidate. Without it the
/// schema held in memory stays the old one.
#[tokio::test]
async fn a_crd_schema_edit_is_reported_and_the_new_schema_is_read_after_invalidate() {
    use futures::StreamExt as _;
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
    use kube::Api;
    use kube::api::{Patch, PatchParams};
    use oxikube_kube::KubeDiscovery;
    use oxikube_ports::{DiscoveryEvent, DiscoveryPort};

    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let id = cluster(&kind);
    let svc = OpenApiSchemas::with_config((*client).clone(), id.clone(), config());
    let discovery = KubeDiscovery::new((*client).clone());
    discovery.discover().await.expect("discover");
    let mut events = discovery.subscribe();

    let crd = TestCrd::create(&client, kind.context.as_str()).await;
    let old = common::wait_until("the new CRD's schema", DEADLINE, || async {
        svc.invalidate(&id).await.expect("invalidate");
        svc.schema_for(&id, &crd.gvk).await.ok()
    })
    .await;
    assert!(!old.has_property("spec"));
    // The creation's own reports are history; only what follows the edit counts.
    tokio::time::sleep(Duration::from_secs(2)).await;
    while tokio::time::timeout(Duration::from_millis(200), events.next())
        .await
        .is_ok()
    {}

    let patch = serde_json::json!({"spec": {"versions": [{
        "name": "v1", "served": true, "storage": true,
        "schema": {"openAPIV3Schema": {
            "type": "object",
            "properties": {"spec": {"type": "object", "properties": {"color": {"type": "string"}}}},
        }},
    }]}});
    Api::<CustomResourceDefinition>::all((*client).clone())
        .patch(&crd.name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .expect("edit the CRD's schema");

    let started = Instant::now();
    loop {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, events.next()).await {
            Ok(Some(DiscoveryEvent::SchemasChanged)) => break,
            Ok(Some(DiscoveryEvent::KindsChanged(change))) => {
                assert!(
                    !change.changed.contains(&crd.gvk),
                    "the kind record did not change: {change:?}"
                );
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => panic!("no SchemasChanged within {DEADLINE:?} of the edit"),
        }
    }

    // As the session does: one invalidate. The server's OpenAPI document may trail the CRD by
    // a moment, so the first lookup can still be answered from the old index; the adapter's
    // re-check while settling must replace it without a second invalidate.
    svc.invalidate(&id).await.expect("invalidate");
    let new = common::wait_until("the edited schema", DEADLINE, || async {
        svc.schema_for(&id, &crd.gvk)
            .await
            .ok()
            .filter(|schema| schema.has_property("spec"))
    })
    .await;
    let color = new
        .properties
        .get("spec")
        .and_then(|s| s.properties.get("color"));
    assert!(color.is_some(), "spec.color is in the schema: {new:?}");

    crd.delete(&client).await;
}
