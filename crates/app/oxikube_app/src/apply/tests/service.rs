//! The pipeline as the editor drives it: dry run, preview, apply, conflicts, the fallback.

use oxikube_domain::{ConflictDetails, ConflictReason, FieldCause, OxiError, ValidationDetails};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;

use super::*;
use crate::apply::{ApplyMethod, FieldConflict};

#[test]
fn the_dry_run_comes_before_the_apply_and_previews_both_sides() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    assert_eq!(t.writes(), [("patch", true)], "the preview writes nothing");
    assert_eq!(preview.method, ApplyMethod::ServerSide);
    assert_eq!(preview.label(), "ConfigMap default/web");
    assert_eq!(preview.live.as_ref().unwrap()["data"]["mode"], "green");
    assert_eq!(preview.result["data"]["mode"], "blue");
    assert_eq!(preview.resource_version.as_deref(), Some("7"));
    assert!(!preview.creates());

    let report = t.apply(&preview).expect("apply");
    assert_eq!(report.method, ApplyMethod::ServerSide);
    assert_eq!(report.message, "Applied ConfigMap default/web");
    // The preview's dry run, the handler's own rehearsal, then the apply.
    assert_eq!(
        t.writes(),
        [("patch", true), ("patch", true), ("patch", false)]
    );
    let writes = t.h.writes("a");
    let ResourceCall::Patch {
        patch,
        options,
        namespace,
        ..
    } = &writes[2]
    else {
        panic!("a patch")
    };
    assert_eq!(
        patch.kind,
        PatchKind::Apply {
            manager: "oxikube".into(),
            force: false
        }
    );
    assert_eq!(options.field_manager.as_deref(), Some("oxikube"));
    assert_eq!(namespace.as_deref(), Some("default"));
    // The body is the manifest's data: no comment, the namespace filled in.
    assert_eq!(patch.body["metadata"]["namespace"], "default");
    assert_eq!(t.port().objects()[0].get_str("/data/mode"), Some("blue"));
}

#[test]
fn a_preview_not_confirmed_applies_nothing() {
    let t = Applying::new(false);
    t.preview(t.request(CONFIGMAP)).expect("preview");
    assert!(t.writes().iter().all(|(_, dry_run)| *dry_run));
    assert!(t.port().objects().is_empty());
}

#[test]
fn the_field_manager_and_the_namespace_come_from_the_request() {
    let t = Applying::new(false);
    let request = t
        .request(CONFIGMAP)
        .managed_by(Some("team-a".into()))
        .in_namespace(Some("shop".into()));
    let preview = t.preview(request).expect("preview");
    assert_eq!(preview.target.namespace(), Some("shop"));
    t.apply(&preview).expect("apply");
    let writes = t.h.writes("a");
    let ResourceCall::Patch { patch, .. } = writes.last().unwrap() else {
        panic!("a patch")
    };
    assert!(matches!(&patch.kind, PatchKind::Apply { manager, .. } if manager == "team-a"));
}

fn ownership_conflict() -> OxiError {
    let cause = |field: &str, manager: &str| FieldCause {
        field: field.into(),
        manager: Some(manager.into()),
        reason: "FieldManagerConflict".into(),
        message: format!("conflict with \"{manager}\""),
    };
    OxiError::conflict("Apply failed with 2 conflicts").with_source(ConflictDetails {
        reason: ConflictReason::FieldOwnership,
        causes: vec![cause(".data.mode", "kubectl"), cause(".data.size", "helm")],
    })
}

#[test]
fn a_conflict_lists_the_managers_and_force_applies_over_them() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    t.port().script().patch.push_err(ownership_conflict());
    let error = t.preview(t.request(CONFIGMAP)).unwrap_err();
    let ApplyError::Conflicts(conflicts) = &error else {
        panic!("conflicts, got {error:?}")
    };
    assert_eq!(
        conflicts,
        &[
            FieldConflict {
                field: ".data.mode".into(),
                manager: Some("kubectl".into()),
                // The newest write of that manager.
                applied_at: Some("2026-10-02T11:00:00Z".into()),
            },
            FieldConflict {
                field: ".data.size".into(),
                manager: Some("helm".into()),
                applied_at: Some("2026-09-30T09:00:00Z".into()),
            },
        ]
    );
    assert_eq!(
        error.to_string(),
        "2 field(s) are managed by kubectl and helm"
    );

    // "Force": preview again with force, then apply.
    let preview = t
        .preview(t.request(CONFIGMAP).forced())
        .expect("forced preview");
    t.apply(&preview).expect("forced apply");
    let forced: Vec<bool> =
        t.h.writes("a")
            .iter()
            .filter_map(|call| match call {
                ResourceCall::Patch { patch, .. } => match patch.kind {
                    PatchKind::Apply { force, .. } => Some(force),
                    _ => None,
                },
                _ => None,
            })
            .collect();
    assert_eq!(forced, [false, true, true, true], "force only once chosen");
}

fn unsupported() -> OxiError {
    OxiError::unsupported("the body of the request was in an unknown format")
}

#[test]
fn a_server_without_apply_gets_a_replace_at_the_reviewed_version() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    t.port().script().patch.push_err(unsupported());
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    assert_eq!(preview.method, ApplyMethod::Replace);
    assert_eq!(preview.resource_version.as_deref(), Some("7"));
    assert_eq!(t.writes(), [("patch", true), ("replace", true)]);

    // Someone changes the object after the review: the reviewed version still goes out.
    t.port().insert(live_configmap("8"));
    t.port().script().patch.push_err(unsupported());
    let report = t.apply(&preview).expect("apply");
    assert_eq!(report.method, ApplyMethod::Replace);
    assert!(report.method.is_fallback());
    assert!(report.message.starts_with("Replaced ConfigMap default/web"));
    let writes = t.h.writes("a");
    assert_eq!(
        t.writes()[2..],
        [("patch", true), ("replace", true), ("replace", false)]
    );
    for replace in &writes[3..] {
        assert_eq!(body(replace)["metadata"]["resourceVersion"], "7");
    }
}

#[test]
fn a_stale_replace_says_the_object_changed() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    t.port().script().patch.push_err(unsupported());
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    t.port().script().patch.push_err(unsupported());
    t.port().script().replace.push_err(
        OxiError::conflict("the object has been modified").with_source(ConflictDetails {
            reason: ConflictReason::StaleVersion,
            causes: Vec::new(),
        }),
    );
    let error = t.apply(&preview).unwrap_err();
    assert!(matches!(error, ApplyError::Stale(_)), "{error:?}");
    assert!(
        t.writes().iter().all(|(_, dry_run)| *dry_run),
        "nothing was written"
    );
}

#[test]
fn a_server_without_apply_creates_a_new_object() {
    let t = Applying::new(false);
    t.port().script().patch.push_err(unsupported());
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    assert_eq!(preview.method, ApplyMethod::Create);
    assert!(preview.creates());
    t.port().script().patch.push_err(unsupported());
    let report = t.apply(&preview).expect("apply");
    assert_eq!(report.method, ApplyMethod::Create);
    assert_eq!(t.port().objects().len(), 1);
    assert!(
        body(&t.h.writes("a")[3])["metadata"]
            .get("resourceVersion")
            .is_none()
    );
}

#[test]
fn a_rejected_field_comes_back_with_its_path() {
    let t = Applying::new(false);
    t.port().script().patch.push_err(
        OxiError::validation("ConfigMap \"web\" is invalid").with_source(ValidationDetails {
            causes: vec![FieldCause {
                field: "data.mode".into(),
                reason: "FieldValueInvalid".into(),
                message: "Invalid value".into(),
                manager: None,
            }],
        }),
    );
    let error = t.preview(t.request(CONFIGMAP)).unwrap_err();
    let ApplyError::Invalid { message, causes } = error else {
        panic!("invalid")
    };
    assert_eq!(message, "ConfigMap \"web\" is invalid");
    assert_eq!(causes[0].field, "data.mode");
}

#[test]
fn a_manifest_that_names_no_single_object_is_refused_before_any_request() {
    let t = Applying::new(false);
    for (text, said) in [
        ("{not yaml", "not valid YAML"),
        ("", "empty"),
        ("kind: ConfigMap\nmetadata: {name: a}\n", "apiVersion"),
        ("apiVersion: v1\nkind: ConfigMap\n", "metadata.name"),
        (
            "apiVersion: v1\nkind: ConfigMap\nmetadata: {name: a}\n---\napiVersion: v1\nkind: ConfigMap\nmetadata: {name: b}\n",
            "2 objects",
        ),
    ] {
        let error = t.preview(t.request(text)).unwrap_err();
        let ApplyError::Invalid { message, .. } = &error else {
            panic!("{text:?}: {error:?}")
        };
        assert!(message.contains(said), "{text:?}: {message}");
    }
    assert!(t.writes().is_empty());
}

#[test]
fn a_secret_is_previewed_and_reported_without_its_values() {
    let t = Applying::new(false);
    let live = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db", "namespace": "default", "resourceVersion": "3"},
        "data": {"password": "b2xkLXBhc3N3b3Jk", "user": "YWRtaW4="}
    }))
    .unwrap();
    t.port().insert(live);
    let manifest = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: db\ndata:\n  password: bmV3LXBhc3N3b3Jk\n  user: YWRtaW4=\n";
    let preview = t.preview(t.request(manifest)).expect("preview");
    let report = t.apply(&preview).expect("apply");
    let shown = format!(
        "{} {} {} {}",
        preview.live.as_ref().unwrap(),
        preview.result,
        report.object,
        report.message
    );
    for value in ["b2xkLXBhc3N3b3Jk", "bmV3LXBhc3N3b3Jk", "YWRtaW4="] {
        assert!(!shown.contains(value), "{value} leaked: {shown}");
    }
    // The preview still tells a changed value from an unchanged one.
    let live = preview.live.as_ref().unwrap();
    assert_eq!(live["data"]["user"], preview.result["data"]["user"]);
    assert_ne!(live["data"]["password"], preview.result["data"]["password"]);
    // The cluster got the real values.
    assert_eq!(
        t.port().objects()[0].get_str("/data/password"),
        Some("bmV3LXBhc3N3b3Jk")
    );
}

#[test]
fn a_cluster_scoped_kind_is_applied_without_a_namespace() {
    let t = Applying::new(false);
    let request = t
        .request("apiVersion: v1\nkind: Namespace\nmetadata:\n  name: shop\n")
        .in_namespace(Some("default".into()));
    let preview = t.preview(request).expect("preview");
    assert_eq!(preview.target.namespace(), None);
    let writes = t.h.writes("a");
    let ResourceCall::Patch {
        namespace, patch, ..
    } = &writes[0]
    else {
        panic!("a patch")
    };
    assert_eq!(namespace, &None);
    assert!(patch.body["metadata"].get("namespace").is_none());
}

#[test]
fn the_body_drops_what_a_server_side_apply_refuses() {
    let t = Applying::new(false);
    let manifest = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web\n  namespace: default\n  resourceVersion: \"5\"\n  managedFields: []\ndata: {a: b}\n";
    t.preview(t.request(manifest)).expect("preview");
    let writes = t.h.writes("a");
    let metadata = &body(&writes[0])["metadata"];
    assert!(metadata.get("resourceVersion").is_none());
    assert!(metadata.get("managedFields").is_none());
}
