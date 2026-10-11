//! Storage view-models over realistic objects.

use serde_json::{Value, json};

use super::*;
use crate::quantity::Quantity;
use crate::resource::Resource;
use crate::view::{ConditionStatus, ViewError};

fn res(json: Value) -> Resource {
    Resource::from_json(json).expect("a valid resource")
}

fn pvc(spec_extra: Value, status: Value) -> Resource {
    let mut spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "resources": {"requests": {"storage": "10Gi"}},
        "storageClassName": "fast",
        "volumeName": "pvc-1",
        "volumeMode": "Filesystem"
    });
    spec.as_object_mut()
        .unwrap()
        .extend(spec_extra.as_object().cloned().unwrap_or_default());
    res(json!({
        "apiVersion": "v1", "kind": "PersistentVolumeClaim",
        "metadata": {"name": "data", "namespace": "shop", "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": spec, "status": status
    }))
}

fn q(text: &str) -> Quantity {
    Quantity::parse(text).unwrap()
}

#[test]
fn a_bound_claim_shows_its_class_size_and_volume() {
    let claim = pvc(
        json!({}),
        json!({"phase": "Bound", "capacity": {"storage": "10Gi"}, "accessModes": ["ReadWriteMany", "ReadOnlyMany"]}),
    );
    let s = PvcSummary::from_resource(&claim).unwrap();
    assert_eq!(s.storage_class.as_deref(), Some("fast"));
    assert_eq!(s.requested, Some(q("10Gi")));
    assert_eq!(s.capacity, Some(q("10Gi")));
    assert_eq!(s.phase, PvcPhase::Bound);
    assert_eq!(access_modes_text(&s.access_modes), "RWX,ROX");
    assert_eq!(s.volume_name.as_deref(), Some("pvc-1"));
    assert_eq!(s.size_text(), "10Gi");
    assert!(!s.resizing());
}

#[test]
fn a_pending_claim_shows_what_was_asked_and_falls_back_to_spec_modes() {
    let s = PvcSummary::from_resource(&pvc(json!({}), json!({"phase": "Pending"}))).unwrap();
    assert_eq!(s.phase, PvcPhase::Pending);
    assert_eq!(s.size_text(), "10Gi");
    assert_eq!(access_modes_text(&s.access_modes), "RWO");
}

#[test]
fn a_resize_in_flight_shows_both_sizes() {
    let claim = pvc(
        json!({"resources": {"requests": {"storage": "20Gi"}}}),
        json!({"phase": "Bound", "capacity": {"storage": "10Gi"},
               "conditions": [{"type": "FileSystemResizePending", "status": "True", "message": "Waiting for the kubelet"}],
               "allocatedResourceStatuses": {"storage": "NodeResizePending"}}),
    );
    let s = PvcSummary::from_resource(&claim).unwrap();
    assert_eq!(s.size_text(), "10Gi → 20Gi");
    assert!(s.resizing());
    assert_eq!(s.current_request(), Some(q("20Gi")));
    assert_eq!(s.resize_status.as_deref(), Some("NodeResizePending"));
    assert_eq!(s.conditions[0].status, ConditionStatus::True);
}

#[test]
fn the_legacy_class_annotation_names_the_class_when_the_spec_does_not() {
    let claim = res(json!({
        "apiVersion": "v1", "kind": "PersistentVolumeClaim",
        "metadata": {"name": "old", "namespace": "x",
                     "annotations": {"volume.beta.kubernetes.io/storage-class": "legacy"}},
        "spec": {}
    }));
    let s = PvcSummary::from_resource(&claim).unwrap();
    assert_eq!(s.storage_class.as_deref(), Some("legacy"));
    assert_eq!(s.size_text(), "");
}

#[test]
fn a_summary_refuses_the_wrong_kind() {
    let pod = res(json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}}));
    assert!(matches!(
        PvcSummary::from_resource(&pod),
        Err(ViewError::WrongKind { .. })
    ));
    assert!(PvSummary::from_resource(&pod).is_err());
    assert!(StorageClassSummary::from_resource(&pod).is_err());
    assert!(VolumeSnapshotSummary::from_resource(&pod).is_err());
}

fn pv(spec: Value, status: Value) -> Resource {
    res(json!({"apiVersion": "v1", "kind": "PersistentVolume",
        "metadata": {"name": "pvc-1", "creationTimestamp": "2026-01-01T00:00:00Z"},
        "spec": spec, "status": status}))
}

#[test]
fn a_volume_shows_capacity_modes_reclaim_and_claim() {
    let volume = pv(
        json!({
            "capacity": {"storage": "100Gi"},
            "accessModes": ["ReadWriteOnce", "ReadWriteOncePod"],
            "persistentVolumeReclaimPolicy": "Delete",
            "claimRef": {"namespace": "shop", "name": "data"},
            "storageClassName": "fast",
            "volumeMode": "Block",
            "csi": {"driver": "ebs.csi.aws.com", "volumeHandle": "vol-0"},
            "mountOptions": ["noatime"],
            "nodeAffinity": {"required": {"nodeSelectorTerms": [
                {"matchExpressions": [
                    {"key": "topology.kubernetes.io/zone", "operator": "In", "values": ["eu-1a", "eu-1b"]},
                    {"key": "kubernetes.io/os", "operator": "Exists"}]}]}}
        }),
        json!({"phase": "Bound"}),
    );
    let s = PvSummary::from_resource(&volume).unwrap();
    assert_eq!(s.capacity, Some(q("100Gi")));
    assert_eq!(access_modes_text(&s.access_modes), "RWO,RWOP");
    assert_eq!(s.reclaim_policy, ReclaimPolicy::Delete);
    assert!(s.reclaim_policy.deletes_data());
    assert_eq!(s.phase, PvPhase::Bound);
    assert_eq!(s.claim.as_ref().unwrap().to_string(), "shop/data");
    let source = s.source.unwrap();
    assert_eq!(&*source.kind, "csi");
    assert_eq!(source.detail.as_deref(), Some("ebs.csi.aws.com"));
    assert_eq!(
        s.node_affinity.lines(),
        ["topology.kubernetes.io/zone In eu-1a,eu-1b AND kubernetes.io/os Exists"]
    );
    assert_eq!(&*s.mount_options[0], "noatime");
}

#[test]
fn a_released_volume_with_no_policy_retains() {
    let volume = pv(
        json!({"capacity": {"storage": "1Gi"}, "hostPath": {"path": "/data"}}),
        json!({"phase": "Released", "reason": "claim gone"}),
    );
    let s = PvSummary::from_resource(&volume).unwrap();
    assert_eq!(s.reclaim_policy, ReclaimPolicy::Retain);
    assert_eq!(s.phase, PvPhase::Released);
    assert_eq!(s.reason.as_deref(), Some("claim gone"));
    assert_eq!(s.source.unwrap().detail.as_deref(), Some("/data"));
    assert!(s.claim.is_none());
}

fn class(annotations: Value, extra: Value) -> Resource {
    let mut json = json!({"apiVersion": "storage.k8s.io/v1", "kind": "StorageClass",
        "metadata": {"name": "fast", "annotations": annotations}, "provisioner": "ebs.csi.aws.com"});
    json.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    res(json)
}

#[test]
fn default_class_is_detected_from_either_annotation() {
    let stable = class(json!({DEFAULT_CLASS_ANNOTATION: "true"}), json!({}));
    let beta = class(json!({BETA_DEFAULT_CLASS_ANNOTATION: "true"}), json!({}));
    let off = class(json!({DEFAULT_CLASS_ANNOTATION: "false"}), json!({}));
    let none = class(json!({}), json!({}));
    assert!(is_default_class(&stable.meta));
    assert!(is_default_class(&beta.meta));
    assert!(!is_default_class(&off.meta));
    assert!(!is_default_class(&none.meta));
    assert!(
        StorageClassSummary::from_resource(&beta)
            .unwrap()
            .is_default
    );
    // The stable annotation says false but the beta one says true: the API server still treats
    // the class as default.
    let mixed = class(
        json!({DEFAULT_CLASS_ANNOTATION: "false", BETA_DEFAULT_CLASS_ANNOTATION: "true"}),
        json!({}),
    );
    assert!(is_default_class(&mixed.meta));
}

#[test]
fn a_class_has_the_api_defaults_and_its_parameters() {
    let bare = StorageClassSummary::from_resource(&class(json!({}), json!({}))).unwrap();
    assert_eq!(bare.reclaim_policy, ReclaimPolicy::Delete);
    assert_eq!(&*bare.binding_mode, "Immediate");
    assert!(!bare.allow_expansion);
    let full = StorageClassSummary::from_resource(&class(
        json!({}),
        json!({"reclaimPolicy": "Retain", "volumeBindingMode": "WaitForFirstConsumer",
               "allowVolumeExpansion": true, "mountOptions": ["debug"],
               "parameters": {"type": "gp3", "encrypted": "true"}}),
    ))
    .unwrap();
    assert_eq!(full.reclaim_policy, ReclaimPolicy::Retain);
    assert_eq!(&*full.binding_mode, "WaitForFirstConsumer");
    assert!(full.allow_expansion);
    assert_eq!(full.parameters[0].0.as_ref(), "encrypted");
    assert_eq!(full.parameters.len(), 2);
    assert_eq!(&*full.mount_options[0], "debug");
}

#[test]
fn snapshots_read_their_source_class_and_readiness() {
    let snap = res(json!({
        "apiVersion": "snapshot.storage.k8s.io/v1", "kind": "VolumeSnapshot",
        "metadata": {"name": "nightly", "namespace": "shop"},
        "spec": {"source": {"persistentVolumeClaimName": "data"}, "volumeSnapshotClassName": "csi-snap"},
        "status": {"readyToUse": true, "restoreSize": "10Gi", "creationTime": "2026-02-01T03:00:00Z",
                   "boundVolumeSnapshotContentName": "snapcontent-1"}
    }));
    let s = VolumeSnapshotSummary::from_resource(&snap).unwrap();
    assert_eq!(s.source_pvc.as_deref(), Some("data"));
    assert_eq!(s.class.as_deref(), Some("csi-snap"));
    assert_eq!(s.ready_to_use, Some(true));
    assert_eq!(s.restore_size, Some(q("10Gi")));
    assert!(s.creation_time.is_some());
    let class = res(json!({
        "apiVersion": "snapshot.storage.k8s.io/v1", "kind": "VolumeSnapshotClass",
        "metadata": {"name": "csi-snap", "annotations": {DEFAULT_SNAPSHOT_CLASS_ANNOTATION: "true"}},
        "driver": "ebs.csi.aws.com", "deletionPolicy": "Delete"
    }));
    let c = VolumeSnapshotClassSummary::from_resource(&class).unwrap();
    assert!(c.is_default);
    assert_eq!(c.driver.as_deref(), Some("ebs.csi.aws.com"));
    assert_eq!(c.deletion_policy.as_deref(), Some("Delete"));
}

#[test]
fn resize_must_grow_and_the_class_must_allow_it() {
    let allowed = Expansion::Allowed;
    let ten = q("10Gi");
    assert_eq!(check_resize(Some(&ten), "20Gi", &allowed), Ok(q("20Gi")));
    assert_eq!(check_resize(Some(&ten), " 11Gi ", &allowed), Ok(q("11Gi")));
    assert_eq!(
        check_resize(Some(&ten), "", &allowed),
        Err(ResizeBlock::Empty)
    );
    assert!(matches!(
        check_resize(Some(&ten), "lots", &allowed),
        Err(ResizeBlock::Invalid(_))
    ));
    assert!(matches!(
        check_resize(Some(&ten), "-5Gi", &allowed),
        Err(ResizeBlock::Invalid(_))
    ));
    // Shrinking, and "the same size", are refused; so is 10240Mi, which is 10Gi.
    for same_or_less in ["5Gi", "10Gi", "10240Mi"] {
        assert!(
            matches!(
                check_resize(Some(&ten), same_or_less, &allowed),
                Err(ResizeBlock::NotLarger { .. })
            ),
            "{same_or_less}"
        );
    }
    let message = check_resize(Some(&ten), "5Gi", &allowed)
        .unwrap_err()
        .to_string();
    assert!(message.contains("cannot shrink"), "{message}");
}

#[test]
fn a_class_that_forbids_expansion_blocks_with_that_reason() {
    let class = StorageClassSummary::from_resource(&class(json!({}), json!({}))).unwrap();
    let expansion = Expansion::of(Some("fast"), Some(&class));
    let err = check_resize(Some(&q("1Gi")), "2Gi", &expansion).unwrap_err();
    assert_eq!(
        err,
        ResizeBlock::ExpansionDisallowed {
            class: "fast".into()
        }
    );
    assert!(err.to_string().contains("allowVolumeExpansion"));
    assert_eq!(
        check_resize(None, "2Gi", &Expansion::of(None, None)),
        Err(ResizeBlock::NoStorageClass)
    );
    // A class that could not be read does not block; the server's dry run decides.
    let unknown = Expansion::of(Some("gone"), None);
    assert_eq!(check_resize(None, "2Gi", &unknown), Ok(q("2Gi")));
    let growing = StorageClassSummary::from_resource(&class_expandable()).unwrap();
    assert_eq!(
        Expansion::of(Some("fast"), Some(&growing)),
        Expansion::Allowed
    );
}

fn class_expandable() -> Resource {
    class(json!({}), json!({"allowVolumeExpansion": true}))
}

fn pod(volumes: Value, phase: &str) -> Resource {
    res(
        json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "web-0", "namespace": "shop"},
        "spec": {"volumes": volumes}, "status": {"phase": phase}}),
    )
}

#[test]
fn a_pod_names_the_claims_it_mounts() {
    let p = pod(
        json!([
            {"name": "data", "persistentVolumeClaim": {"claimName": "data"}},
            {"name": "again", "persistentVolumeClaim": {"claimName": "data"}},
            {"name": "cfg", "configMap": {"name": "cfg"}},
            {"name": "scratch", "ephemeral": {"volumeClaimTemplate": {"spec": {}}}},
            {"name": "other", "persistentVolumeClaim": {"claimName": "logs", "readOnly": true}}
        ]),
        "Running",
    );
    let claims: Vec<String> = pod_claims(&p).iter().map(|c| c.to_string()).collect();
    assert_eq!(claims, ["data", "web-0-scratch", "logs"]);
    assert!(pod_is_active(&p));
    assert!(!pod_is_active(&pod(json!([]), "Succeeded")));
    assert!(!pod_is_active(&pod(json!([]), "Failed")));
    assert!(pod_claims(&pod(json!([]), "Pending")).is_empty());
}

#[test]
fn access_modes_and_policies_round_trip() {
    for (text, short) in [
        ("ReadWriteOnce", "RWO"),
        ("ReadOnlyMany", "ROX"),
        ("ReadWriteMany", "RWX"),
        ("ReadWriteOncePod", "RWOP"),
        ("Future", "Future"),
    ] {
        let mode = AccessMode::parse(text);
        assert_eq!((mode.as_str(), mode.short()), (text, short));
    }
    assert_eq!(ReclaimPolicy::parse("Recycle").to_string(), "Recycle");
}
