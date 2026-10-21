//! Builders for release objects and the reader under test.

use std::io::Write as _;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_helm::NativeHelmReader;
use oxikube_ports::helm::ReleaseRef;
use oxikube_testkit::fakes::{FakeResourcePort, ResourceCall};
use serde_json::{Value, json};

/// The storage driver of a synthetic release object.
#[derive(Clone, Copy, Debug)]
pub enum Driver {
    Secret,
    ConfigMap,
}

pub fn cluster() -> ClusterId {
    ClusterId::new("kubeconfig-test", &ContextName::new("kind-helm"))
}

pub fn release_ref(namespace: &str, name: &str) -> ReleaseRef {
    ReleaseRef::new(cluster(), namespace, name)
}

/// A reader over a fake holding `objects`; the fake is returned for call assertions.
pub fn reader(
    objects: impl IntoIterator<Item = Resource>,
) -> (NativeHelmReader, Arc<FakeResourcePort>) {
    let fake = Arc::new(FakeResourcePort::new().with_objects(objects));
    (NativeHelmReader::new(cluster(), fake.clone()), fake)
}

/// A recorded object from `tests/fixtures/`.
pub fn fixture(text: &str) -> Resource {
    Resource::from_json(serde_json::from_str(text).expect("fixture JSON")).expect("fixture")
}

/// Every recorded helm 4.3.1 object (captured from kind; see `tests/fixtures/README.md`).
pub fn recorded() -> Vec<Resource> {
    [
        include_str!("../fixtures/helm4.3.1-secret-web.v1.json"),
        include_str!("../fixtures/helm4.3.1-secret-web.v2.json"),
        include_str!("../fixtures/helm4.3.1-secret-web.v3.json"),
        include_str!("../fixtures/helm4.3.1-secret-gone.v1.json"),
        include_str!("../fixtures/helm4.3.1-configmap-web.cm-app.v1.json"),
        include_str!("../fixtures/helm4.3.1-configmap-web.cm-app.v2.json"),
    ]
    .into_iter()
    .map(fixture)
    .collect()
}

/// The namespace the recorded objects live in.
pub const RECORDED_NS: &str = "oxi-test-helmcap12395";

/// Helm's release JSON for a synthetic revision.
pub fn record(name: &str, namespace: &str, revision: u32, status: &str) -> Value {
    json!({
        "name": name,
        "namespace": namespace,
        "version": revision,
        "info": {
            "first_deployed": "2026-01-01T00:00:00Z",
            "last_deployed": format!("2026-01-01T00:{:02}:00Z", revision % 60),
            "description": format!("{status} at revision {revision}"),
            "status": status,
            "notes": format!("Thanks for installing {name}."),
        },
        "chart": {
            "metadata": {"name": "demo", "version": "1.2.3", "appVersion": "2.0.0"},
            "values": {"replicas": 1, "image": {"repository": "web", "tag": "1.0"}},
        },
        "config": {"image": {"tag": "1.1"}},
        "manifest": "---\napiVersion: v1\nkind: ConfigMap\n",
    })
}

/// Helm's encoding: gzip, then base64.
pub fn helm_encode(record: &Value) -> String {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(record.to_string().as_bytes()).unwrap();
    STANDARD.encode(gz.finish().unwrap())
}

/// A release object holding `data` verbatim as its `data.release`.
pub fn object_with_data(
    driver: Driver,
    namespace: &str,
    name: &str,
    revision: u32,
    status: &str,
    data: &str,
) -> Resource {
    let object = format!("sh.helm.release.v1.{name}.v{revision}");
    let metadata = json!({
        "name": object,
        "namespace": namespace,
        "uid": format!("uid-{namespace}-{object}"),
        "resourceVersion": "1",
        "labels": {
            "owner": "helm", "name": name, "status": status,
            "version": revision.to_string(), "modifiedAt": "1767225600",
        },
    });
    let json = match driver {
        Driver::Secret => json!({
            "apiVersion": "v1", "kind": "Secret", "type": "helm.sh/release.v1",
            "metadata": metadata, "data": {"release": STANDARD.encode(data)},
        }),
        Driver::ConfigMap => json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": metadata, "data": {"release": data},
        }),
    };
    Resource::from_json(json).unwrap()
}

/// A release object storing `record`.
pub fn object(driver: Driver, record: &Value) -> Resource {
    let name = record["name"].as_str().unwrap();
    let namespace = record["namespace"].as_str().unwrap();
    let revision = record["version"].as_u64().unwrap() as u32;
    let status = record["info"]["status"].as_str().unwrap();
    object_with_data(
        driver,
        namespace,
        name,
        revision,
        status,
        &helm_encode(record),
    )
}

/// A synthetic revision stored as a Secret.
pub fn secret(namespace: &str, name: &str, revision: u32, status: &str) -> Resource {
    object(Driver::Secret, &record(name, namespace, revision, status))
}

/// How many objects the reader fetched (decoded) so far.
pub fn gets(fake: &FakeResourcePort) -> usize {
    fake.recorded_calls()
        .iter()
        .filter(|c| matches!(c, ResourceCall::Get { .. }))
        .count()
}

/// How many full (payload-carrying) lists the reader made: must stay zero.
pub fn full_lists(fake: &FakeResourcePort) -> usize {
    fake.recorded_calls()
        .iter()
        .filter(|c| matches!(c, ResourceCall::List { .. }))
        .count()
}
