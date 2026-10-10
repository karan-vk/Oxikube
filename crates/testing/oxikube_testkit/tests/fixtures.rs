//! Fixture and builder checks: every manifest parses into a `Resource` and its view-model,
//! the set on disk matches `fixtures::ALL`, the Helm release Secret decodes, and the
//! builders agree with the matching fixtures field by field.

use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::event::{Event, EventType};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::json::JsonRef;
use oxikube_domain::{
    ContainerSummary, CronJobSummary, JobSummary, NodeSummary, PodSummary, Resource,
    WorkloadSummary,
};
use oxikube_testkit::builders::{
    PodBuilder, daemonset, deployment, job, node, pod, replicaset, statefulset,
};
use oxikube_testkit::fixtures;
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Every `*.json` under `fixtures/`, relative, skipping the kind manifests and the
/// OpenAPI schema documents (which are not `Resource`s; see
/// `every_openapi_fixture_is_registered_and_shaped`).
fn json_files_on_disk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "cluster" || n == "metrics-server" || n == "openapi")
            {
                continue;
            }
            json_files_on_disk(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            let rel = path.strip_prefix(root).unwrap();
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn every_fixture_on_disk_is_registered_and_there_are_at_least_30() {
    let root = fixtures_dir();
    let mut on_disk = Vec::new();
    json_files_on_disk(&root, &root, &mut on_disk);
    on_disk.sort();
    let mut registered: Vec<String> = fixtures::ALL.iter().map(|s| (*s).to_owned()).collect();
    registered.sort();
    assert_eq!(on_disk, registered);
    assert!(registered.len() >= 30, "only {} fixtures", registered.len());
}

#[test]
fn every_fixture_parses_into_a_resource_and_its_view_model() {
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"));
    for path in fixtures::ALL {
        let res = fixtures::try_load(path).unwrap_or_else(|e| panic!("{e}"));
        assert!(!res.name().is_empty(), "{path}");
        assert!(res.meta.uid.is_some(), "{path}: fixtures carry a uid");
        assert!(
            res.meta.creation.is_some(),
            "{path}: fixtures carry a creationTimestamp"
        );
        let kind = &*res.kind.kind;
        match kind {
            "Pod" => {
                PodSummary::from_resource(&res).unwrap();
                ContainerSummary::list_from_resource(&res).unwrap();
            }
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" => {
                WorkloadSummary::from_resource(&res).unwrap();
            }
            "Job" => {
                JobSummary::from_resource(&res).unwrap();
            }
            "CronJob" => {
                CronJobSummary::from_resource(&res).unwrap();
            }
            "Node" => {
                NodeSummary::from_resource(&res).unwrap();
            }
            "Event" => {
                Event::from_json(&cluster, &res.to_value())
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
            }
            _ => {}
        }
    }
}

#[test]
fn every_openapi_fixture_is_registered_and_shaped() {
    let root = fixtures_dir().join("openapi");
    let mut on_disk = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            on_disk.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    on_disk.sort();
    let mut registered: Vec<String> = fixtures::openapi::ALL
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    registered.sort();
    assert_eq!(on_disk, registered);
    for name in fixtures::openapi::ALL {
        let document = fixtures::openapi::json(name);
        assert!(
            document
                .get("components")
                .and_then(|c| c.get("schemas"))
                .and_then(Value::as_object)
                .is_some_and(|schemas| !schemas.is_empty()),
            "{name}: an OpenAPI group document with components.schemas"
        );
    }
    // The documents the story calls out resolve through the domain lookup.
    let deployment = oxikube_domain::schema::root_schema_for(
        &fixtures::openapi::deployment_apps_v1(),
        &"apps/v1/Deployment".parse().unwrap(),
    )
    .expect("deployment schema");
    assert!(deployment.has_property("spec"));
    let widget = oxikube_domain::schema::root_schema_for(
        &fixtures::openapi::widget_crd(),
        &"example.com/v1/Widget".parse().unwrap(),
    )
    .expect("widget schema");
    assert!(
        widget
            .properties
            .get("spec")
            .and_then(|spec| spec.properties.get("config"))
            .is_some_and(|config| config.xk8s.preserve_unknown_fields)
    );
}
#[test]
fn pod_fixtures_have_the_expected_status_strings() {
    let cases = [
        ("pods/pending.json", "Pending", "0/1"),
        ("pods/container-creating.json", "ContainerCreating", "0/1"),
        ("pods/running.json", "Running", "1/1"),
        ("pods/running-restarted.json", "Running", "1/1"),
        ("pods/succeeded.json", "Completed", "0/1"),
        ("pods/failed.json", "Error", "0/1"),
        ("pods/evicted.json", "Evicted", "0/1"),
        ("pods/crashloop.json", "CrashLoopBackOff", "0/1"),
        ("pods/image-pull-backoff.json", "ImagePullBackOff", "0/1"),
        ("pods/err-image-pull.json", "ErrImagePull", "0/1"),
        ("pods/oom-killed.json", "OOMKilled", "0/1"),
        ("pods/init.json", "Init:1/2", "0/1"),
        ("pods/terminating.json", "Terminating", "1/1"),
        ("pods/node-lost.json", "NodeLost", "0/1"),
        ("pods/sidecar.json", "Running", "2/2"),
    ];
    let pods_on_file = fixtures::ALL
        .iter()
        .filter(|p| p.starts_with("pods/"))
        .count();
    assert_eq!(
        cases.len(),
        pods_on_file,
        "every pod fixture has an expected status"
    );
    for (path, status, ready) in cases {
        let s = PodSummary::from_resource(&fixtures::load(path)).unwrap();
        assert_eq!(&*s.status, status, "{path}");
        assert_eq!(s.ready_display(), ready, "{path}");
    }
    let crash = PodSummary::from_resource(&fixtures::pod_crashloop()).unwrap();
    assert_eq!(crash.restarts, 5);
    let restarted = PodSummary::from_resource(&fixtures::pod_running_restarted()).unwrap();
    assert_eq!(restarted.restarts, 3);
}

#[test]
fn workload_node_and_event_fixtures_read_as_expected() {
    let d = WorkloadSummary::from_resource(&fixtures::deployment_progressing()).unwrap();
    assert_eq!((d.desired, d.ready, d.available), (3, 2, 2));
    let n = NodeSummary::from_resource(&fixtures::node_cordoned()).unwrap();
    assert_eq!(&*n.status, "Ready,SchedulingDisabled");
    let cp = NodeSummary::from_resource(&fixtures::node_control_plane()).unwrap();
    assert_eq!(cp.roles_display(), "control-plane");
    let mp = NodeSummary::from_resource(&fixtures::node_memory_pressure()).unwrap();
    assert_eq!(
        mp.problems().map(|c| &*c.kind).collect::<Vec<_>>(),
        vec!["MemoryPressure"]
    );
    let cj = CronJobSummary::from_resource(&fixtures::cronjob()).unwrap();
    assert_eq!((&*cj.schedule, cj.active), ("0 3 * * *", 1));

    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"));
    let warn = Event::from_json(&cluster, &fixtures::event_core_warning().to_value()).unwrap();
    assert_eq!(
        (warn.event_type, &*warn.reason, warn.count),
        (EventType::Warning, "BackOff", 42)
    );
    assert_eq!(&*warn.regarding.name, "web-crashloop");
    let v1 = Event::from_json(&cluster, &fixtures::event_events_v1().to_value()).unwrap();
    assert_eq!(&*v1.reason, "ScalingReplicaSet");
    assert_eq!(
        v1.related.map(|r| r.name.to_string()).as_deref(),
        Some("web-5d8c7b9f4")
    );

    let crd = fixtures::widget_crd();
    let widget = fixtures::widget();
    assert_eq!(crd.get_str("/spec/group"), Some(&*widget.kind.group));
    assert_eq!(crd.get_str("/spec/names/kind"), Some(&*widget.kind.kind));
}

#[test]
fn helm_release_secret_decodes_base64_base64_gzip_json() {
    let secret = fixtures::helm_release_secret();
    assert_eq!(secret.get_str("/type"), Some("helm.sh/release.v1"));
    assert_eq!(secret.meta.labels.get("owner").map(|v| &**v), Some("helm"));
    let data = secret.get_str("/data/release").unwrap();
    let helm_encoded = STANDARD.decode(data).unwrap();
    let gzipped = STANDARD.decode(helm_encoded).unwrap();
    let mut json = String::new();
    flate2::read::GzDecoder::new(&gzipped[..])
        .read_to_string(&mut json)
        .unwrap();
    let release: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(release["name"], "web");
    assert_eq!(release["version"], 2);
    assert_eq!(release["info"]["status"], "deployed");
    assert_eq!(release["chart"]["metadata"]["version"], "1.2.3");
}

#[test]
fn secret_fixtures_hold_only_dummy_values() {
    let secret = fixtures::secret();
    let data = secret.get("/data").and_then(JsonRef::as_object).unwrap();
    for (key, value) in data.iter() {
        let decoded = STANDARD.decode(value.as_str().unwrap()).unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert!(
            text.starts_with("dummy") || text.starts_with("not-a-real"),
            "{key} must be obvious dummy data, got {text:?}"
        );
    }
}

// --- builders vs fixtures -----------------------------------------------------------------

fn same_pod(builder: PodBuilder, fixture: &Resource) {
    let built = builder.build();
    let (a, b) = (
        PodSummary::from_resource(&built).unwrap(),
        PodSummary::from_resource(fixture).unwrap(),
    );
    assert_eq!(a, b, "{}", fixture.name());
    let containers = |r: &Resource| {
        ContainerSummary::list_from_resource(r)
            .unwrap()
            .into_iter()
            .map(|c| {
                (
                    c.name,
                    c.kind,
                    c.image,
                    c.ready,
                    c.started,
                    c.restarts,
                    c.state.label().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        containers(&built),
        containers(fixture),
        "{}",
        fixture.name()
    );
    assert_eq!(built.kind, fixture.kind);
    assert_eq!(built.meta.namespace, fixture.meta.namespace);
    assert_eq!(built.meta.creation, fixture.meta.creation);
    assert_eq!(built.meta.deletion, fixture.meta.deletion);
}

#[test]
fn pod_builders_match_pod_fixtures() {
    same_pod(
        pod().name("web-pending").pending(),
        &fixtures::pod_pending(),
    );
    same_pod(
        pod().name("web-creating").container_creating(),
        &fixtures::pod_container_creating(),
    );
    same_pod(
        pod().name("web-running").running(),
        &fixtures::pod_running(),
    );
    same_pod(
        pod().name("web-restarted").running().restarts(3),
        &fixtures::pod_running_restarted(),
    );
    same_pod(
        pod().name("web-succeeded").succeeded(),
        &fixtures::pod_succeeded(),
    );
    same_pod(pod().name("web-failed").failed(), &fixtures::pod_failed());
    same_pod(
        pod().name("web-crashloop").crash_loop(),
        &fixtures::pod_crashloop(),
    );
    same_pod(
        pod().name("web-imagepull").image_pull_backoff(),
        &fixtures::pod_image_pull_backoff(),
    );
    same_pod(
        pod().name("web-oomkilled").oom_killed(),
        &fixtures::pod_oom_killed(),
    );
    same_pod(pod().name("web-init").init(1, 2), &fixtures::pod_init());
    same_pod(
        pod().name("web-terminating").running().terminating(),
        &fixtures::pod_terminating(),
    );
    same_pod(
        pod().name("web-nodelost").node_lost(),
        &fixtures::pod_node_lost(),
    );
}

#[test]
fn workload_and_job_builders_match_fixtures() {
    let w = |r: Resource| WorkloadSummary::from_resource(&r).unwrap();
    assert_eq!(
        w(deployment().replicas(3).build()),
        w(fixtures::deployment_ready())
    );
    assert_eq!(
        w(deployment().replicas(3).ready(2).build()),
        w(fixtures::deployment_progressing())
    );
    assert_eq!(
        w(statefulset().replicas(3).partition(1).build()),
        w(fixtures::statefulset())
    );
    assert_eq!(w(daemonset().ready(1).build()), w(fixtures::daemonset()));
    assert_eq!(
        w(replicaset().replicas(3).build()),
        w(fixtures::replicaset())
    );

    let j = |r: Resource| JobSummary::from_resource(&r).unwrap();
    assert_eq!(
        j(job().complete().completions(3).build()),
        j(fixtures::job_complete())
    );
    assert_eq!(
        j(job().name("migrate-failed").failed().build()),
        j(fixtures::job_failed())
    );
}

#[test]
fn node_builders_match_node_fixtures() {
    // Conditions carry reasons and heartbeat times only the fixtures have; compare what the
    // conditions mean (readiness, problems) instead of their full text.
    let n = |r: Resource| {
        let mut s = NodeSummary::from_resource(&r).unwrap();
        let ready = s.is_ready();
        let problems: Vec<String> = s.problems().map(|c| c.kind.to_string()).collect();
        s.conditions.clear();
        (s, ready, problems)
    };
    assert_eq!(n(node().build()), n(fixtures::node_ready()));
    assert_eq!(
        n(node().name("worker-2").not_ready().build()),
        n(fixtures::node_not_ready())
    );
    assert_eq!(
        n(node().name("worker-3").cordoned().build()),
        n(fixtures::node_cordoned())
    );
    assert_eq!(
        n(node()
            .name("oxikube-control-plane")
            .role("control-plane")
            .build()),
        n(fixtures::node_control_plane())
    );
    assert_eq!(
        n(node()
            .name("worker-4")
            .condition("MemoryPressure", true)
            .build()),
        n(fixtures::node_memory_pressure())
    );
}
