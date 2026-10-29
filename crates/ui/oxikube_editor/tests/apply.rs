//! Applying the buffer (E10-S08) in a workspace, end to end over the real `ApplyService`,
//! `CommandBus` and `MutationGuard`, with the testkit's fake cluster ports on the deterministic
//! runtime: the key opens the server's dry run as a diff preview, Apply sends a server-side
//! apply, a read-only cluster is refused before any request, a rejected field gets a squiggle,
//! a conflict lists its managers and Force applies over them, a buffer with errors is refused,
//! and the palette's `resource::Apply` runs the same flow.

use gpui::{AppContext as _, TestAppContext};
use oxikube_app::ApplyMethod;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::command::CommandId;
use oxikube_domain::{ConflictDetails, ConflictReason, FieldCause, OxiError, ValidationDetails};
use oxikube_editor::view::{ApplyPreviewDialog, ApplyStage, ConflictDialog};
use oxikube_ports::PatchKind;
use oxikube_testkit::fakes::ResourceCall;
use oxikube_workspace::test_support::TestModal;

#[path = "apply_common/mod.rs"]
mod common;

use common::*;

#[gpui::test]
fn the_key_dry_runs_shows_the_diff_and_apply_sends_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();

    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview opened");
    assert_eq!(f.writes(), [("patch", true)], "only the dry run so far");
    let (title, description, added) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog.description(),
            dialog.diff().counts().1,
        )
    });
    assert_eq!(title, "Create ConfigMap shop/web?");
    assert!(
        description.contains("Server-side apply as oxikube"),
        "{description}"
    );
    assert!(added > 0);
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Reviewing
    );

    f.confirm(&dialog);
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    // The preview's dry run, the guard's before it asks (E19-S07), the handler's, the apply.
    assert_eq!(
        f.writes(),
        [
            ("patch", true),
            ("patch", true),
            ("patch", true),
            ("patch", false)
        ]
    );
    let ResourceCall::Patch { patch, .. } = &f.resources.mutating_calls()[3] else {
        panic!("a patch")
    };
    assert_eq!(
        patch.kind,
        PatchKind::Apply {
            manager: "oxikube".into(),
            force: false
        }
    );
    assert_eq!(f.resources.objects().len(), 1);
    assert_eq!(f.notice(&editor), "Applied ConfigMap shop/web");
    assert!(!f.vcx.read(|cx| editor.read(cx).is_dirty(cx)), "clean");
    let toasts = f.vcx.read(|cx| {
        f.workspace
            .read(cx)
            .toast_layer()
            .read(cx)
            .visible()
            .into_iter()
            .map(|t| t.message.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(toasts, ["Applied ConfigMap shop/web"]);
    let audit: Vec<(bool, AuditOutcome)> = f
        .state
        .audit_log()
        .iter()
        .map(|r| (r.dry_run, r.outcome))
        .collect();
    // The preview, the guard's dry run (E19-S07), the apply.
    assert_eq!(
        audit,
        [
            (true, AuditOutcome::Succeeded),
            (true, AuditOutcome::Succeeded),
            (false, AuditOutcome::Succeeded)
        ]
    );
}

#[gpui::test]
fn cancel_sends_nothing_more(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    dialog.update(&mut f.vcx, |dialog, cx| dialog.cancel(cx));
    f.vcx.run_until_parked();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert_eq!(f.writes(), [("patch", true)]);
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Idle
    );
}

impl Fixture {
    /// Another modal replaces the open one, as the palette's key does over a dialog.
    fn replace_modal(&mut self) {
        let workspace = self.workspace.clone();
        workspace.update_in(&mut self.vcx, |workspace, window, cx| {
            let other = cx.new(|cx| TestModal::new("other", cx));
            workspace.show_modal(other, window, cx);
        });
        self.vcx.run_until_parked();
    }
}

#[gpui::test]
fn a_preview_replaced_by_another_modal_leaves_the_editor_idle(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
    assert_eq!(f.stage(&editor), ApplyStage::Reviewing);

    f.replace_modal();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
    assert_eq!(f.writes(), [("patch", true)], "nothing applied");
    // Apply works again.
    f.workspace.update_in(&mut f.vcx, |workspace, window, cx| {
        workspace.hide_modal(window, cx)
    });
    editor.update_in(&mut f.vcx, |view, window, cx| view.begin_apply(window, cx));
    f.vcx.run_until_parked();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
}

#[gpui::test]
fn a_conflict_list_replaced_by_another_modal_leaves_the_editor_idle(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(
        OxiError::conflict("Apply failed with 1 conflict").with_source(ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: vec![FieldCause {
                field: ".data.mode".into(),
                manager: Some("kubectl".into()),
                reason: "FieldManagerConflict".into(),
                message: "conflict with \"kubectl\"".into(),
            }],
        }),
    );
    f.press_apply();
    assert!(f.modal::<ConflictDialog>().is_some());
    assert_eq!(f.stage(&editor), ApplyStage::Reviewing);

    f.replace_modal();
    assert!(f.modal::<ConflictDialog>().is_none());
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
}

#[gpui::test]
fn a_read_only_cluster_is_refused_before_any_request(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.editor_with(CONFIGMAP);
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert!(f.resources.recorded_calls().is_empty());
    assert_eq!(
        f.notice(&editor),
        "kind-oxikube is read-only: the manifest was not applied"
    );
    assert_eq!(f.state.audit_log()[0].outcome, AuditOutcome::Denied);
    // Read-only mode lifted: the same key works.
    f.sessions.set_read_only(&cluster(), false).unwrap();
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
}

#[gpui::test]
fn a_rejected_field_gets_a_squiggle_on_its_key(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(
        OxiError::validation("ConfigMap \"web\" is invalid").with_source(ValidationDetails {
            causes: vec![FieldCause {
                field: "data.mode".into(),
                reason: "FieldValueInvalid".into(),
                message: "Invalid value: \"blue\"".into(),
                manager: None,
            }],
        }),
    );
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    let (squiggles, problems) = f.vcx.read(|cx| {
        let view = editor.read(cx);
        (
            view.model().server_problems().to_vec(),
            view.model().problems(),
        )
    });
    assert_eq!(squiggles.len(), 1);
    assert_eq!(&CONFIGMAP[squiggles[0].range.clone()], "mode");
    assert_eq!(squiggles[0].message, "data.mode: Invalid value: \"blue\"");
    assert_eq!(problems.errors, 1);
    assert!(
        f.notice(&editor)
            .starts_with("The server rejected the manifest: ConfigMap \"web\" is invalid"),
        "{}",
        f.notice(&editor)
    );
}

#[gpui::test]
fn a_conflict_lists_its_managers_and_force_applies_over_them(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(
        OxiError::conflict("Apply failed with 1 conflict").with_source(ConflictDetails {
            reason: ConflictReason::FieldOwnership,
            causes: vec![FieldCause {
                field: ".data.mode".into(),
                manager: Some("kubectl".into()),
                reason: "FieldManagerConflict".into(),
                message: "conflict with \"kubectl\"".into(),
            }],
        }),
    );
    f.press_apply();
    let conflicts = f.modal::<ConflictDialog>().expect("the conflict list");
    let managers = f.vcx.read(|cx| {
        let dialog = conflicts.read(cx);
        assert_eq!(dialog.conflicts()[0].field, ".data.mode");
        dialog.managers().join(",")
    });
    assert_eq!(managers, "kubectl");

    conflicts.update_in(&mut f.vcx, |dialog, window, cx| dialog.force(window, cx));
    f.vcx.run_until_parked();
    let preview = f.modal::<ApplyPreviewDialog>().expect("the forced preview");
    let forced = f.vcx.read(|cx| preview.read(cx).preview().request.force);
    assert!(forced);
    f.confirm(&preview);
    let forces: Vec<bool> = f
        .resources
        .mutating_calls()
        .iter()
        .filter_map(|call| match call {
            ResourceCall::Patch { patch, .. } => match patch.kind {
                PatchKind::Apply { force, .. } => Some(force),
                _ => None,
            },
            _ => None,
        })
        .collect();
    // The forced preview, the guard's dry run, the handler's, the apply.
    assert_eq!(forces, [false, true, true, true, true]);
    assert_eq!(f.notice(&editor), "Applied ConfigMap shop/web");
}

#[gpui::test]
fn a_buffer_with_errors_is_refused_before_the_server_sees_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with("apiVersion: v1\nkind: ConfigMap\nmetadata: [\n");
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert!(f.resources.recorded_calls().is_empty());
    assert!(
        f.notice(&editor).starts_with("Fix the"),
        "{}",
        f.notice(&editor)
    );
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Idle
    );
}

#[gpui::test]
fn the_palettes_apply_runs_the_editors_flow(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.editor_with(CONFIGMAP);
    let ran = f.vcx.update(|window, cx| {
        let surface = oxikube_workspace::command_surface::focused(window, cx).expect("a surface");
        assert!(surface.own_commands.contains(&CommandId::RESOURCE_APPLY));
        assert_eq!(surface.target.cluster, Some(cluster()));
        oxikube_workspace::command_surface::run_on_focused(
            CommandId::RESOURCE_APPLY,
            Vec::new(),
            window,
            cx,
        )
    });
    assert!(ran);
    f.vcx.run_until_parked();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
}

#[gpui::test]
fn a_server_without_apply_is_previewed_as_a_replace(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.resources.insert(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "web", "namespace": "shop", "resourceVersion": "41"},
            "data": {"mode": "green"}
        }))
        .unwrap(),
    );
    let editor = f.editor_with(CONFIGMAP);
    f.resources.script().patch.push_err(OxiError::unsupported(
        "the body of the request was in an unknown format",
    ));
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    let (method, description) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (dialog.preview().method, dialog.description())
    });
    assert_eq!(method, ApplyMethod::Replace);
    assert!(
        description.contains("replaced at resourceVersion 41"),
        "{description}"
    );
    // 415 again for the guard's dry run (E19-S07) and for the handler's.
    for _ in 0..2 {
        f.resources.script().patch.push_err(OxiError::unsupported(
            "the body of the request was in an unknown format",
        ));
    }
    f.confirm(&dialog);
    let replaced = f.resources.mutating_calls();
    let ResourceCall::Replace {
        object, options, ..
    } = replaced.last().unwrap()
    else {
        panic!("a replace: {replaced:?}")
    };
    assert!(!options.dry_run);
    assert_eq!(object["metadata"]["resourceVersion"], "41");
    assert!(f.notice(&editor).starts_with("Replaced ConfigMap shop/web"));
}

#[gpui::test]
fn an_api_without_dry_run_shows_a_banner_and_still_applies(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.resources.insert(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "web", "namespace": "shop", "resourceVersion": "41"},
            "data": {"mode": "green"}
        }))
        .unwrap(),
    );
    let editor = f.editor_with(CONFIGMAP);
    // An aggregated API that serves writes but no dry run (E19-S07): every dry run is refused.
    let no_dry_run =
        || OxiError::unsupported("the server does not allow this method on the requested resource");
    f.resources.script().patch.push_err(no_dry_run());
    f.resources.script().replace.push_err(no_dry_run());
    f.press_apply();
    let dialog = f
        .modal::<ApplyPreviewDialog>()
        .expect("the preview opens anyway");
    let (banner, description) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (dialog.no_preview_banner(), dialog.description())
    });
    let banner = banner.expect("a banner in place of the diff");
    assert!(
        banner.contains("does not support dry run") && banner.contains("does not allow"),
        "{banner}"
    );
    assert!(
        description.starts_with("The server cannot dry-run"),
        "{description}"
    );
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        f.vcx.debug_bounds("apply-preview-no-preview").is_some(),
        "the banner is drawn"
    );
    assert!(
        f.resources
            .mutating_calls()
            .iter()
            .all(ResourceCall::is_dry_run)
    );

    // The guard's dry run and the handler's are refused the same way; the apply is sent.
    for _ in 0..2 {
        f.resources.script().patch.push_err(no_dry_run());
        f.resources.script().replace.push_err(no_dry_run());
    }
    f.confirm(&dialog);
    assert_eq!(f.writes().last(), Some(&("patch", false)));
    assert_eq!(f.notice(&editor), "Applied ConfigMap shop/web");
}
