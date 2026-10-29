//! The happy path: what the copy writes, and what it leaves out.

use serde_json::json;

use super::*;

#[test]
fn the_copy_is_sanitised_renamed_and_placed_in_the_target_namespace() {
    let t = Copying::new(false);
    let request = t.request(&deployment(), "prod").renamed("web-copy");
    let preview = t.preview(request).expect("preview");

    assert_eq!(preview.target.namespace(), Some("prod"));
    assert_eq!(&*preview.target.name, "web-copy");
    assert!(!preview.exists());
    assert!(
        t.target_writes().is_empty(),
        "a preview writes nothing for real"
    );

    let outcome = t.copy(&preview, None).expect("copy");
    assert!(
        matches!(&outcome, CopyOutcome::Copied { created: true, .. }),
        "{outcome:?}"
    );
    assert_eq!(t.target_writes(), [("patch", "web-copy".to_owned())]);

    let stored = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "web-copy")
        .unwrap();
    let object = stored.to_value();
    let metadata = &object["metadata"];
    assert_eq!(metadata["namespace"], "prod");
    for dropped in [
        "uid",
        "resourceVersion",
        "generation",
        "creationTimestamp",
        "selfLink",
        "ownerReferences",
        "finalizers",
        "managedFields",
    ] {
        assert!(
            metadata.get(dropped).is_none(),
            "{dropped} should be gone: {metadata}"
        );
    }
    assert!(object.get("status").is_none());
    // Labels and the other annotations stay; the last-applied annotation and the revision go.
    assert_eq!(metadata["labels"], json!({"app": "web"}));
    assert_eq!(metadata["annotations"], json!({"team": "payments"}));
    assert_eq!(object["spec"]["replicas"], 2);

    // The report names the dropped fields by path, never a value.
    assert!(
        preview.removed.iter().any(|path| path == "status"),
        "{:?}",
        preview.removed
    );
    assert!(preview.removed.iter().any(|path| path == "metadata.uid"));
    assert!(!preview.removed.iter().any(|path| path.contains("7a1f")));
    assert_eq!(t.source_writes(), 0, "the source is only read");
}

#[test]
fn a_cluster_scoped_kind_is_copied_without_a_namespace() {
    let t = Copying::new(false);
    let class = json!({"apiVersion": "v1", "kind": "PersistentVolume",
        "metadata": {"name": "pv-1", "uid": "u"}, "spec": {"capacity": {"storage": "1Gi"}}});
    let request = t.request(&class, "ignored");
    assert_eq!(request.target.namespace(), None);
    let preview = t.preview(request).expect("preview");
    t.copy_with(&preview, None, ReviewAnswer::typed("pv-1"))
        .expect("copy");
    let stored = &t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "pv-1")
        .unwrap();
    assert!(stored.to_value()["metadata"].get("namespace").is_none());
}

#[test]
fn a_service_loses_its_allocated_addresses_and_a_claim_its_volume() {
    let t = Copying::new(false);
    let service = json!({"apiVersion": "v1", "kind": "Service",
        "metadata": {"name": "web", "namespace": "shop"},
        "spec": {"type": "NodePort", "clusterIP": "10.0.0.7", "clusterIPs": ["10.0.0.7"],
            "ports": [{"port": 80, "nodePort": 31000}], "selector": {"app": "web"}}});
    let preview = t.preview(t.request(&service, "prod")).expect("preview");
    t.copy(&preview, None).expect("copy");
    let spec = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "web")
        .unwrap()
        .to_value()["spec"]
        .clone();
    assert!(spec.get("clusterIP").is_none(), "{spec}");
    assert!(spec.get("clusterIPs").is_none());
    assert!(spec["ports"][0].get("nodePort").is_none());
    assert_eq!(spec["selector"], json!({"app": "web"}));

    let claim = json!({"apiVersion": "v1", "kind": "PersistentVolumeClaim",
        "metadata": {"name": "data", "namespace": "shop"},
        "spec": {"volumeName": "pv-9", "resources": {"requests": {"storage": "1Gi"}}}});
    let preview = t.preview(t.request(&claim, "prod")).expect("preview");
    t.copy(&preview, None).expect("copy");
    let spec = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "data")
        .unwrap()
        .to_value()["spec"]
        .clone();
    assert!(spec.get("volumeName").is_none(), "{spec}");
}

#[test]
fn a_job_loses_its_generated_selector() {
    let t = Copying::new(false);
    let job = json!({"apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "migrate", "namespace": "shop"},
        "spec": {"selector": {"matchLabels": {"controller-uid": "u1"}},
            "template": {"metadata": {"labels": {"controller-uid": "u1", "job-name": "migrate"}},
                "spec": {"containers": [{"name": "m", "image": "busybox"}], "restartPolicy": "Never"}}}});
    let preview = t.preview(t.request(&job, "prod")).expect("preview");
    t.copy(&preview, None).expect("copy");
    let spec = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "migrate")
        .unwrap()
        .to_value()["spec"]
        .clone();
    assert!(spec.get("selector").is_none(), "{spec}");
    assert!(
        spec["template"]["metadata"]["labels"]
            .get("controller-uid")
            .is_none()
    );
}

#[test]
fn the_preview_diffs_against_the_object_at_the_target() {
    let t = Copying::new(false);
    let target = json!({"apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "prod"}, "data": {"mode": "green"}});
    t.target().insert(Resource::from_json(target).unwrap());
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    assert!(preview.exists());
    assert_eq!(preview.change(), crate::apply::Change::Update);
    assert_eq!(preview.live.as_ref().unwrap()["data"]["mode"], "green");
    assert_eq!(preview.result["data"]["mode"], "blue");
}

#[test]
fn a_secret_is_copied_but_its_values_are_masked_in_the_preview() {
    let t = Copying::new(false);
    let secret = json!({"apiVersion": "v1", "kind": "Secret", "type": "Opaque",
        "metadata": {"name": "db", "namespace": "shop"},
        "data": {"password": "aHVudGVyMi12ZXJ5LXNlY3JldA=="}});
    let preview = t.preview(t.request(&secret, "prod")).expect("preview");
    let shown = preview.result.to_string();
    assert!(!shown.contains("aHVudGVy"), "{shown}");
    assert!(!shown.contains("hunter2"));
    t.copy_with(&preview, None, ReviewAnswer::click())
        .expect("copy");
    // The target holds the real value: it is the copy.
    let stored = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "db")
        .unwrap()
        .to_value();
    assert_eq!(stored["data"]["password"], "aHVudGVyMi12ZXJ5LXNlY3JldA==");
}
