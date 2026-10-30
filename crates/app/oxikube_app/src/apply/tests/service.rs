//! The pipeline as the editor drives it: dry run, preview, apply, conflicts, the fallback.

use oxikube_domain::{
    ConflictDetails, ConflictReason, ErrorKind, FieldCause, OxiError, ValidationDetails,
};
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

    let (calls, records) = (t.calls().len(), t.h.audit().len());
    let report = t.apply(&preview).expect("apply");
    assert_eq!(report.method, ApplyMethod::ServerSide);
    assert_eq!(report.message, "Applied ConfigMap default/web");
    // The preview's dry run stands in for the guard's before it asks (E19-S07) and for the
    // handler's own: the click sends the apply alone, no read, one audit record.
    assert_eq!(t.writes(), [("patch", true), ("patch", false)]);
    assert_eq!(t.calls().len() - calls, 1, "{:?}", &t.calls()[calls..]);
    assert_eq!(t.h.audit().len() - records, 1);
    assert!(!t.h.audit().last().expect("the apply").dry_run);
    let writes = t.h.writes("a");
    let ResourceCall::Patch {
        patch,
        options,
        namespace,
        ..
    } = &writes[1]
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
    // The forced preview, then the apply.
    assert_eq!(forced, [false, true, true], "force only once chosen");
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

    // Someone changes the object after the review: the reviewed version still goes out. The
    // server answers 415 to the apply (the preview was its dry run).
    t.port().insert(live_configmap("8"));
    t.port().script().patch.push_err(unsupported());
    let report = t.apply(&preview).expect("apply");
    assert_eq!(report.method, ApplyMethod::Replace);
    assert!(report.method.is_fallback());
    assert!(report.message.starts_with("Replaced ConfigMap default/web"));
    let writes = t.h.writes("a");
    assert_eq!(t.writes()[2..], [("patch", false), ("replace", false)]);
    assert_eq!(body(&writes[3])["metadata"]["resourceVersion"], "7");
}

#[test]
fn a_stale_replace_says_the_object_changed() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    t.port().script().patch.push_err(unsupported());
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    // The object changed since the review: the replace at the reviewed version is refused.
    t.port().script().patch.push_err(unsupported());
    t.port().script().replace.push_err(
        OxiError::conflict("the object has been modified").with_source(ConflictDetails {
            reason: ConflictReason::StaleVersion,
            causes: Vec::new(),
        }),
    );
    let error = t.apply(&preview).unwrap_err();
    assert!(matches!(error, ApplyError::Stale(_)), "{error:?}");
    let writes = t.h.writes("a");
    assert_eq!(t.writes()[2..], [("patch", false), ("replace", false)]);
    assert_eq!(body(&writes[3])["metadata"]["resourceVersion"], "7");
    assert_eq!(
        t.port().objects()[0].get_str("/metadata/resourceVersion"),
        Some("7"),
        "the server refused it: nothing changed"
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
    assert_eq!(t.writes()[2..], [("patch", false), ("create", false)]);
    assert!(
        body(&t.h.writes("a")[3])["metadata"]
            .get("resourceVersion")
            .is_none()
    );
}

#[test]
fn a_create_reviewed_is_not_turned_into_a_replace_of_an_object_made_since() {
    let t = Applying::new(false);
    t.port().script().patch.push_err(unsupported());
    let preview = t.preview(t.request(CONFIGMAP)).expect("preview");
    assert!(preview.creates());
    // Someone else creates the object before the user confirms.
    t.port().insert(live_configmap("9"));
    t.port().script().patch.push_err(unsupported());
    t.port().script().create.push_err(
        OxiError::conflict("configmaps \"web\" already exists").with_source(ConflictDetails {
            reason: ConflictReason::AlreadyExists,
            causes: Vec::new(),
        }),
    );
    let error = t.apply(&preview).unwrap_err();
    assert!(matches!(error, ApplyError::Stale(_)), "{error:?}");
    // The handler holds to the create the user reviewed: no replace, and the server refused the
    // create.
    assert_eq!(
        t.writes()[2..],
        [("patch", false), ("create", false)],
        "no replace"
    );
    assert_eq!(t.port().objects()[0].get_str("/data/mode"), Some("green"));
}

#[test]
fn octal_numbers_are_read_as_kubectl_reads_them() {
    let t = Applying::new(false);
    let manifest = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web\n  annotations: {mode: \"0644\"}\nspec:\n  defaultMode: 0644\n  short: 012\n  plain: 644\n";
    t.preview(t.request(manifest)).expect("preview");
    let writes = t.h.writes("a");
    let sent = body(&writes[0]);
    assert_eq!(sent["spec"]["defaultMode"], json!(420));
    assert!(sent["spec"]["defaultMode"].is_u64() || sent["spec"]["defaultMode"].is_i64());
    assert_eq!(sent["spec"]["short"], json!(10));
    assert_eq!(sent["spec"]["plain"], json!(644));
    assert_eq!(
        sent["metadata"]["annotations"]["mode"], "0644",
        "a string stays one"
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

/// An aggregated API that serves writes but no dry run (E19-S07): 405 for every dry run.
fn no_dry_run() -> OxiError {
    OxiError::unsupported("the server does not allow this method on the requested resource")
}

#[test]
fn an_api_without_dry_run_is_previewed_as_no_preview_and_still_applied() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    // The preview: SSA and the replace fallback both refuse a dry run.
    t.port().script().patch.push_err(no_dry_run());
    t.port().script().replace.push_err(no_dry_run());
    let preview = t
        .preview(t.request(CONFIGMAP))
        .expect("a preview without a diff");
    assert_eq!(
        preview.no_preview.as_deref(),
        Some("the server does not allow this method on the requested resource")
    );
    assert_eq!(preview.label(), "ConfigMap default/web");
    assert!(preview.live.is_none() && preview.result.is_null());
    assert!(!preview.creates(), "nothing is known about the object");
    assert!(t.port().objects()[0].get_str("/data/mode") == Some("green"));

    // The apply: the guard's dry run fails the same way (no preview), the handler sends no dry
    // run of its own, and the apply is sent.
    t.port().script().patch.push_err(no_dry_run());
    t.port().script().replace.push_err(no_dry_run());
    let report = t
        .apply(&preview)
        .expect("a missing preview never blocks the apply");
    assert_eq!(report.method, ApplyMethod::ServerSide);
    assert_eq!(t.writes().last(), Some(&("patch", false)));
    assert_eq!(t.port().objects()[0].get_str("/data/mode"), Some("blue"));
}

#[test]
fn a_webhook_that_refuses_dry_runs_is_no_preview_not_an_invalid_manifest() {
    let t = Applying::new(false);
    let refusal =
        || OxiError::validation("admission webhook \"audit.example.com\" does not support dry run");
    t.port().script().patch.push_err(refusal());
    let preview = t
        .preview(t.request(CONFIGMAP))
        .expect("a preview without a diff");
    assert!(
        preview
            .no_preview
            .as_deref()
            .is_some_and(|reason| reason.contains("does not support dry run")),
        "{preview:?}"
    );
    // The guard's dry run is refused; the handler sends none of its own, and the apply itself
    // goes through.
    t.port().script().patch.push_err(refusal());
    let report = t.apply(&preview).expect("applied");
    assert_eq!(report.method, ApplyMethod::ServerSide);
    assert_eq!(t.port().objects().len(), 1);
}

#[test]
fn a_server_that_refuses_every_dry_run_still_gets_the_confirmed_apply() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    t.port().refuse_dry_runs(
        ErrorKind::Unsupported,
        "dry run is not supported by this API",
    );
    let preview = t
        .preview(t.request(CONFIGMAP))
        .expect("a preview without a diff");
    assert!(preview.no_preview.is_some(), "{preview:?}");
    let report = t
        .apply(&preview)
        .expect("a missing preview never blocks the apply");
    assert_eq!(report.method, ApplyMethod::ServerSide);
    assert_eq!(t.port().objects()[0].get_str("/data/mode"), Some("blue"));
    assert_eq!(
        t.writes().iter().filter(|(_, dry_run)| !dry_run).count(),
        1,
        "one apply: {:?}",
        t.writes()
    );
}
