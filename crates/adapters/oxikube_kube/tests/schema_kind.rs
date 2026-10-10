//! Kind integration for E10-S01: `SchemaPort` against the real `/openapi/v3` of the
//! kind cluster: a built-in kind (Deployment) and the sample CRD (`Widget`).
//! Needs `cargo xtask kind-up` (installs the `widgets.test.oxikube.dev` CRD) and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::SchemaType;
use oxikube_kube::{OpenApiConfig, OpenApiSchemas};
use oxikube_ports::SchemaPort;
use oxikube_testkit::FakeFsPort;

use common::DEADLINE;

fn cluster(kind: &common::Kind) -> ClusterId {
    ClusterId::new("kind-test", &kind.context)
}

#[tokio::test]
async fn deployment_and_widget_schemas_come_from_the_server() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let client = kind.admin_client().await;
    let svc = OpenApiSchemas::with_config(
        (*client).clone(),
        Arc::new(FakeFsPort::new()),
        PathBuf::from("/openapi-kind-cache"),
        OpenApiConfig {
            request_timeout: Duration::from_secs(30),
        },
    );
    let started = Instant::now();

    let deployment = svc
        .schema_for(&cluster(&kind), &Gvk::new("apps", "v1", "Deployment"))
        .await
        .expect("deployment schema");
    let replicas = deployment
        .properties
        .get("spec")
        .and_then(|spec| spec.properties.get("replicas"))
        .expect("spec.replicas");
    assert!(
        replicas.types.contains(&SchemaType::Integer)
            || replicas.types.contains(&SchemaType::String),
        "replicas is IntOrString on a real server: {:?}",
        replicas.types
    );

    let widget = svc
        .schema_for(
            &cluster(&kind),
            &Gvk::new("test.oxikube.dev", "v1", "Widget"),
        )
        .await
        .expect("widget schema");
    assert!(
        widget.has_property("spec"),
        "the sample CRD resolves by group-version-kind"
    );

    let again = svc
        .schema_for(&cluster(&kind), &Gvk::new("apps", "v1", "Deployment"))
        .await
        .expect("cached deployment schema");
    assert!(Arc::ptr_eq(&deployment, &again));
    assert!(
        started.elapsed() < DEADLINE * 4,
        "first schemas arrive promptly"
    );
}
