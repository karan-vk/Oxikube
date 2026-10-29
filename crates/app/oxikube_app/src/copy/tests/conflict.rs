//! An object that exists at the target: overwrite (with and without force), skip, rename.

use oxikube_domain::{ConflictDetails, ConflictReason, FieldCause, OxiError};
use oxikube_ports::PatchKind;

use super::*;
use crate::apply::FieldConflict;

fn existing(t: &Copying) {
    t.target().insert(
        Resource::from_json(json!({"apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "settings", "namespace": "prod", "resourceVersion": "3"},
            "data": {"mode": "green"}}))
        .unwrap(),
    );
}

fn ownership_conflict() -> OxiError {
    let cause = |field: &str, manager: &str| FieldCause {
        field: field.into(),
        manager: Some(manager.into()),
        reason: "FieldManagerConflict".into(),
        message: String::new(),
    };
    OxiError::conflict("Apply failed with 1 conflict").with_source(ConflictDetails {
        reason: ConflictReason::FieldOwnership,
        causes: vec![cause(".data.mode", "kubectl")],
    })
}

#[test]
fn an_existing_object_needs_a_choice_and_nothing_is_written_without_one() {
    let t = Copying::new(false);
    existing(&t);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    assert!(preview.exists());
    let error = t.copy(&preview, None).unwrap_err();
    assert!(matches!(error, CopyError::Exists(_)), "{error:?}");
    assert!(t.target_writes().is_empty());
}

#[test]
fn overwrite_server_side_applies_without_force() {
    let t = Copying::new(false);
    existing(&t);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    let outcome = t
        .copy(&preview, Some(CopyChoice::Overwrite { force: false }))
        .expect("overwrite");
    assert!(
        matches!(outcome, CopyOutcome::Copied { created: false, .. }),
        "{outcome:?}"
    );
    let writes = t.h.writes("b");
    let real = writes
        .iter()
        .find(|call| !call.is_dry_run())
        .expect("a write");
    let ResourceCall::Patch { patch, .. } = real else {
        panic!("a patch")
    };
    assert!(
        matches!(&patch.kind, PatchKind::Apply { manager, force } if manager == "oxikube" && !force),
        "{:?}",
        patch.kind
    );
    let stored = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "settings")
        .unwrap();
    assert_eq!(stored.to_value()["data"]["mode"], "blue");
}

#[test]
fn other_managers_fields_need_force_which_the_preview_announces() {
    let t = Copying::new(false);
    existing(&t);
    // The first dry run (no force) hits the conflict; the preview retries with force to still
    // show the diff, and says which fields clash.
    t.target().script().patch.push_err(ownership_conflict());
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    assert_eq!(
        preview.conflicts,
        [FieldConflict {
            field: ".data.mode".into(),
            manager: Some("kubectl".into()),
            applied_at: None
        }]
    );
    assert!(preview.exists());

    // Overwrite without force is refused by the server with the same conflict.
    t.target().script().patch.push_err(ownership_conflict());
    let error = t
        .copy(&preview, Some(CopyChoice::Overwrite { force: false }))
        .unwrap_err();
    assert!(matches!(error, CopyError::Conflicts(_)), "{error:?}");
    assert!(t.target_writes().is_empty());

    // With force it goes through, as a forced server-side apply.
    t.copy(&preview, Some(CopyChoice::Overwrite { force: true }))
        .expect("forced");
    let writes = t.h.writes("b");
    let forced = writes.iter().filter(|call| !call.is_dry_run()).any(|call| {
        matches!(call, ResourceCall::Patch { patch, .. } if matches!(&patch.kind, PatchKind::Apply { force: true, .. }))
    });
    assert!(forced);
}

#[test]
fn skip_writes_nothing_and_the_target_is_untouched() {
    let t = Copying::new(false);
    existing(&t);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    let before = t.h.audit().len();
    let outcome = t.copy(&preview, Some(CopyChoice::Skip)).expect("skip");
    assert!(
        matches!(outcome, CopyOutcome::Skipped { .. }),
        "{outcome:?}"
    );
    assert!(t.target_writes().is_empty());
    assert_eq!(t.h.audit().len(), before, "a skip dispatches nothing");
    let stored = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "settings")
        .unwrap();
    assert_eq!(stored.to_value()["data"]["mode"], "green");
}

#[test]
fn rename_validates_the_name_then_copies_under_it() {
    let t = Copying::new(false);
    existing(&t);
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    for bad in [
        "",
        "Settings",
        "-copy",
        "has space",
        "dots.in.label-",
        &"a".repeat(254),
    ] {
        let error = t
            .copy(&preview, Some(CopyChoice::Rename(bad.to_owned())))
            .unwrap_err();
        assert!(
            matches!(error, CopyError::InvalidName(_)),
            "{bad:?}: {error:?}"
        );
    }
    assert!(t.target_writes().is_empty());

    let outcome = t
        .copy(&preview, Some(CopyChoice::Rename("settings-copy".into())))
        .expect("rename");
    let CopyOutcome::Copied {
        target, created, ..
    } = outcome
    else {
        panic!("copied")
    };
    assert!(created);
    assert_eq!(&*target.name, "settings-copy");
    assert_eq!(t.target_writes(), [("patch", "settings-copy".to_owned())]);
    // The original is as it was.
    let original = t
        .target()
        .objects()
        .into_iter()
        .find(|o| o.name() == "settings")
        .unwrap();
    assert_eq!(original.to_value()["data"]["mode"], "green");
}

#[test]
fn a_rename_onto_another_existing_object_asks_again() {
    let t = Copying::new(false);
    existing(&t);
    t.target().insert(
        Resource::from_json(json!({"apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "settings-copy", "namespace": "prod"}}))
        .unwrap(),
    );
    let preview = t.preview(t.request(&configmap(), "prod")).expect("preview");
    let error = t
        .copy(&preview, Some(CopyChoice::Rename("settings-copy".into())))
        .unwrap_err();
    assert!(matches!(error, CopyError::Exists(_)), "{error:?}");
    assert!(t.target_writes().is_empty());
}

#[test]
fn a_namespace_name_must_be_a_label() {
    let t = Copying::new(false);
    let namespace = json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "shop"}});
    let preview = t
        .preview(CopyRequest::to(source_ref(&namespace), id("b"), None))
        .expect("preview");
    let error = t
        .copy_with(
            &preview,
            Some(CopyChoice::Rename("a.b".into())),
            ReviewAnswer::typed("a.b"),
        )
        .unwrap_err();
    assert!(matches!(error, CopyError::InvalidName(_)), "{error:?}");
}
