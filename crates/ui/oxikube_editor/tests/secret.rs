//! Editing a Secret decoded and creating one (E12-S11), end to end in a workspace over the real
//! `ApplyService`, `CommandBus` and `MutationGuard` on the testkit's fake cluster ports: the
//! buffer holds plaintext, the server gets base64, the preview and the logs hold no value, the
//! saved layout holds nothing, and a read-only cluster is refused before any request.

use std::io::Write;
use std::sync::{Arc, Mutex};

use gpui::{Entity, TestAppContext};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{FieldCause, OxiError, Resource, ValidationDetails};
use oxikube_editor::open::{MASKED, MASKED_CODE, ResourceLoad};
use oxikube_editor::view::{
    ApplyPreviewDialog, ApplyStage, CreateSecretDialog, EditorRequest, ManifestEditor, SecretKind,
};
use oxikube_testkit::fakes::ResourceCall;
use oxikube_ui::editor::EditorApi as _;
use serde_json::{Value, json};

#[path = "apply_common/mod.rs"]
mod common;

use common::*;

const PASSWORD: &str = "hunter2-plaintext";
const PASSWORD_B64: &str = "aHVudGVyMi1wbGFpbnRleHQ=";
const ROTATED: &str = "rotated-plaintext";
const ROTATED_B64: &str = "cm90YXRlZC1wbGFpbnRleHQ=";

fn secret_ref() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Secret"), "shop", "db")
}

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db", "namespace": "shop", "resourceVersion": "4"},
        "type": "Opaque",
        "data": {"password": PASSWORD_B64, "blob": "//4="}
    }))
    .unwrap()
}

impl Fixture {
    fn open_decoded(&mut self) -> Entity<ManifestEditor> {
        self.resources.insert(secret());
        self.sink
            .send(EditorRequest::OpenDecoded {
                target: secret_ref(),
            })
            .unwrap();
        self.vcx.run_until_parked();
        let editor = self.only_editor();
        self.vcx
            .executor()
            .advance_clock(oxikube_editor::view::VALIDATION_DEBOUNCE);
        self.vcx.run_until_parked();
        editor
    }

    fn only_editor(&mut self) -> Entity<ManifestEditor> {
        let mut editors = self
            .vcx
            .read(|cx| self.workspace.read(cx).items_of_type::<ManifestEditor>());
        assert_eq!(editors.len(), 1, "one editor");
        editors.remove(0)
    }

    fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx
            .update(|window, cx| code.update(cx, |code, cx| code.api(window, cx).text()))
            .to_string()
    }

    fn set_text(&mut self, editor: &Entity<ManifestEditor>, text: &str) {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).set_text(text));
        });
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(oxikube_editor::view::VALIDATION_DEBOUNCE);
        self.vcx.run_until_parked();
    }

    fn problems(&mut self, editor: &Entity<ManifestEditor>) -> (usize, usize) {
        self.vcx.read(|cx| {
            let problems = editor.read(cx).model().problems();
            (problems.errors, problems.warnings)
        })
    }

    /// The bodies of the patches the cluster got, dry runs included.
    fn patch_bodies(&self) -> Vec<Value> {
        self.resources
            .mutating_calls()
            .into_iter()
            .filter_map(|call| match call {
                ResourceCall::Patch { patch, .. } => Some(patch.body),
                _ => None,
            })
            .collect()
    }
}

#[gpui::test]
fn edit_decoded_shows_plaintext_and_binary_as_a_size(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    let text = f.text(&editor);
    assert!(text.contains(&format!("password: {PASSWORD}")), "{text}");
    assert!(text.contains("blob: (binary, 2 bytes)"), "{text}");
    assert!(
        !text.contains(PASSWORD_B64) && !text.contains("//4="),
        "no base64: {text}"
    );
    assert!(!text.contains(MASKED));
    let (decoded, load) = f.vcx.read(|cx| {
        let binding = editor.read(cx).binding().expect("a binding");
        (binding.is_decoded(), binding.load().clone())
    });
    assert!(decoded);
    assert_eq!(load, ResourceLoad::Ready);
    assert_eq!(
        f.problems(&editor),
        (0, 0),
        "plaintext is valid: nothing masked"
    );
}

#[gpui::test]
fn applying_re_encodes_to_base64_keeps_binary_and_hides_values_in_the_preview(
    cx: &mut TestAppContext,
) {
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    let text = f.text(&editor).replace(PASSWORD, ROTATED);
    f.set_text(&editor, &text);

    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview opened");
    // The dry run got base64: the plaintext never left the editor.
    let dry = f.patch_bodies();
    assert_eq!(dry.len(), 1, "only the dry run so far");
    assert_eq!(dry[0]["data"]["password"], ROTATED_B64);
    assert_eq!(
        dry[0]["data"]["blob"], "//4=",
        "binary carried through unchanged"
    );
    let wire = dry[0].to_string();
    assert!(
        !wire.contains(ROTATED) && !wire.contains(PASSWORD),
        "{wire}"
    );
    // The preview shows which key changes and never to what.
    let shown = f.vcx.read(|cx| format!("{:?}", dialog.read(cx).diff()));
    for leaked in [ROTATED, PASSWORD, ROTATED_B64, PASSWORD_B64] {
        assert!(
            !shown.contains(leaked),
            "{leaked} is in the preview: {shown}"
        );
    }

    f.confirm(&dialog);
    let bodies = f.patch_bodies();
    assert_eq!(bodies.len(), 3, "dry run, dry run, apply");
    assert_eq!(bodies[2]["data"]["password"], ROTATED_B64);
    let stored = f.resources.objects().remove(0).to_value();
    assert_eq!(stored["data"]["password"], ROTATED_B64);
    assert_eq!(stored["data"]["blob"], "//4=");
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
    assert!(
        !f.vcx.read(|cx| editor.read(cx).is_dirty(cx)),
        "clean after the apply"
    );
}

#[gpui::test]
fn an_untouched_decoded_buffer_changes_nothing(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    let (changed, _) = f.vcx.read(|cx| dialog.read(cx).diff().counts());
    assert_eq!(changed, 0, "re-encoding the plaintext gives the same data");
    let dry = f.patch_bodies();
    assert_eq!(dry[0]["data"]["password"], PASSWORD_B64);
    drop(editor);
}

#[gpui::test]
fn the_diff_panel_compares_a_decoded_secret_as_the_server_would_get_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    editor.update_in(&mut f.vcx, |editor, window, cx| {
        editor.toggle_diff(window, cx)
    });
    f.vcx
        .executor()
        .advance_clock(oxikube_editor::view::DIFF_DEBOUNCE);
    f.vcx.run_until_parked();
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert!(
        diff.is_empty(),
        "plaintext against base64 is not a change: {diff:?}"
    );

    let text = f.text(&editor).replace(PASSWORD, ROTATED);
    f.set_text(&editor, &text);
    f.vcx
        .executor()
        .advance_clock(oxikube_editor::view::DIFF_DEBOUNCE);
    f.vcx.run_until_parked();
    let diff = f.vcx.read(|cx| editor.read(cx).diff()).expect("computed");
    assert_eq!(diff.counts(), (1, 1), "only the key that was edited");
    let shown = format!("{diff:?}");
    assert!(
        !shown.contains(ROTATED) && !shown.contains(ROTATED_B64),
        "{shown}"
    );
}

#[gpui::test]
fn a_rejection_of_the_confirmed_apply_is_placed_in_the_text_typed(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    let text = f.text(&editor).replace(PASSWORD, ROTATED);
    f.set_text(&editor, &text);
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the dry run passed");
    // The real apply is refused (a webhook the dry run does not run).
    f.resources.script().patch.push_err(
        OxiError::validation("Secret \"db\" is invalid").with_source(ValidationDetails {
            causes: vec![FieldCause {
                field: "data.password".into(),
                reason: "FieldValueInvalid".into(),
                message: "too short".into(),
                manager: None,
            }],
        }),
    );
    f.confirm(&dialog);
    let buffer = f.text(&editor);
    let squiggles = f
        .vcx
        .read(|cx| editor.read(cx).model().server_problems().to_vec());
    assert_eq!(squiggles.len(), 1, "{squiggles:?}");
    assert_eq!(
        &buffer[squiggles[0].range.clone()],
        "password",
        "the key of the buffer, not of the base64 text sent"
    );
}

#[gpui::test]
fn hiding_the_values_masks_them_and_blocks_the_apply(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    let id = editor.entity_id();
    editor.update_in(&mut f.vcx, |editor, window, cx| {
        editor.toggle_secret_mask(window, cx)
    });
    f.vcx.run_until_parked();
    let masked = f.text(&editor);
    assert!(masked.contains(MASKED), "{masked}");
    assert!(!masked.contains(PASSWORD));
    f.vcx
        .executor()
        .advance_clock(oxikube_editor::view::VALIDATION_DEBOUNCE);
    f.vcx.run_until_parked();
    assert_eq!(
        f.problems(&editor).0,
        2,
        "each placeholder is an error ({MASKED_CODE})"
    );

    f.press_apply();
    assert!(
        f.modal::<ApplyPreviewDialog>().is_none(),
        "nothing to review"
    );
    assert!(f.patch_bodies().is_empty(), "nothing was sent");

    // Revealing again returns to the decoded text.
    editor.update_in(&mut f.vcx, |editor, window, cx| {
        editor.toggle_secret_mask(window, cx)
    });
    f.vcx.run_until_parked();
    assert!(f.text(&editor).contains(&format!("password: {PASSWORD}")));
    let _ = id;
}

#[gpui::test]
fn a_read_only_cluster_opens_it_read_only_and_refuses_the_apply(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.open_decoded();
    assert!(
        f.vcx
            .read(|cx| editor.read(cx).editor().read(cx).is_read_only())
    );
    f.press_apply();
    assert!(
        f.patch_bodies().is_empty(),
        "no request reached the cluster"
    );
}

#[gpui::test]
fn nothing_decoded_reaches_the_layout_the_audit_or_the_logs(cx: &mut TestAppContext) {
    let logs = capture_logs();
    let mut f = setup(cx, false);
    let editor = f.open_decoded();
    let text = f.text(&editor).replace(PASSWORD, ROTATED);
    f.set_text(&editor, &text);

    // The saved layout holds no editor and no value.
    let saved = f.vcx.read(|cx| {
        serde_json::to_string(&f.workspace.read(cx).serialize_layout(cx)).expect("layout")
    });
    for leaked in [PASSWORD, ROTATED, PASSWORD_B64, ROTATED_B64, "Secret/db"] {
        assert!(
            !saved.contains(leaked),
            "{leaked} reached the layout: {saved}"
        );
    }

    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    f.confirm(&dialog);

    // The audit records name the object and the field manager, never the manifest.
    let audit = f.state.audit_log();
    assert_eq!(audit.len(), 2);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Succeeded));
    // Applied as `resource::Apply`: risk low, so a simple confirmation (the dry-run preview).
    assert!(audit.iter().all(|r| &*r.cmd == "resource::Apply"));
    let meta = oxikube_domain::command::lookup(oxikube_domain::command::CommandId::RESOURCE_APPLY)
        .expect("declared");
    assert_eq!(meta.risk, Some(oxikube_domain::Risk::Low));
    assert_eq!(meta.confirm, oxikube_domain::ConfirmTier::Simple);
    let audit_text = format!("{audit:?}");
    for leaked in [PASSWORD, ROTATED, PASSWORD_B64, ROTATED_B64] {
        assert!(
            !audit_text.contains(leaked),
            "{leaked} is in the audit: {audit_text}"
        );
    }
    // Neither does anything logged while the buffer was opened, edited and applied.
    let logged = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        !logged.is_empty(),
        "the capture works: something was logged"
    );
    for leaked in [PASSWORD, ROTATED, PASSWORD_B64, ROTATED_B64] {
        assert!(!logged.contains(leaked), "{leaked} was logged: {logged}");
    }
}

/// Logged once by [`capture_logs`], to show that the capture works.
const MARKER: &str = "log capture started";

/// Everything logged on this thread from now on, as text (the deterministic runtime runs the
/// tasks of a test on the test's thread).
fn capture_logs() -> Arc<Mutex<Vec<u8>>> {
    #[derive(Clone)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
        type Writer = Sink;
        fn make_writer(&'a self) -> Sink {
            self.clone()
        }
    }
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(Sink(buffer.clone()))
        .finish();
    // Held for the test: dropped with the test's thread-local.
    let guard = tracing::subscriber::set_default(subscriber);
    std::mem::forget(guard);
    tracing::info!("{MARKER}");
    buffer
}

// --- create ----------------------------------------------------------------------------------

impl Fixture {
    fn create_dialog(&mut self) -> Entity<CreateSecretDialog> {
        self.sink
            .send(EditorRequest::CreateSecret { cluster: None })
            .unwrap();
        self.vcx.run_until_parked();
        self.modal::<CreateSecretDialog>()
            .expect("the dialog opened")
    }
}

#[gpui::test]
fn create_secret_opens_the_dialog_in_the_selected_namespace(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    let kind = f.vcx.read(|cx| dialog.read(cx).kind());
    assert_eq!(kind, SecretKind::Opaque);
    let namespace = f.vcx.read(|cx| match dialog.read(cx).draft(cx) {
        oxikube_app::NewSecret::Opaque { namespace, .. } => namespace,
        other => panic!("{other:?}"),
    });
    assert_eq!(namespace, "default", "all namespaces are selected: default");
}

#[gpui::test]
fn an_invalid_secret_marks_its_fields_and_opens_nothing(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.fill_identity("Bad Name", "default", window, cx);
        dialog.fill_row(0, "bad key", "x", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    let fields: Vec<_> = f
        .vcx
        .read(|cx| dialog.read(cx).problems().iter().map(|p| p.field).collect());
    assert_eq!(
        fields,
        [
            oxikube_app::NewSecretField::Name,
            oxikube_app::NewSecretField::Key(0)
        ]
    );
    assert!(f.modal::<CreateSecretDialog>().is_some(), "still open");
    assert!(f.vcx.read(|cx| {
        f.workspace
            .read(cx)
            .items_of_type::<ManifestEditor>()
            .is_empty()
    }));
    assert!(f.patch_bodies().is_empty());
}

#[gpui::test]
fn a_tls_pair_is_validated_as_pem_before_an_editor_opens(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.select_kind(SecretKind::Tls, cx);
        dialog.fill_identity("web-tls", "default", window, cx);
        dialog.fill_tls("not a certificate", "not a key", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    let fields: Vec<_> = f
        .vcx
        .read(|cx| dialog.read(cx).problems().iter().map(|p| p.field).collect());
    assert_eq!(
        fields,
        [
            oxikube_app::NewSecretField::Certificate,
            oxikube_app::NewSecretField::PrivateKey
        ]
    );
    assert!(f.modal::<CreateSecretDialog>().is_some());
}

#[gpui::test]
fn a_created_opaque_secret_opens_as_text_and_applies_as_base64(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.fill_identity("app-creds", "shop", window, cx);
        dialog.fill_row(0, "password", PASSWORD, window, cx);
        dialog.fill_row(1, "user", "admin", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    assert!(
        f.modal::<CreateSecretDialog>().is_none(),
        "the dialog closed"
    );

    let editor = f.only_editor();
    let text = f.text(&editor);
    assert!(text.contains("kind: Secret"), "{text}");
    assert!(text.contains(&format!("password: {PASSWORD}")), "{text}");
    assert!(text.contains("name: app-creds"), "{text}");
    assert!(!text.contains(PASSWORD_B64), "text, not base64: {text}");

    f.vcx
        .executor()
        .advance_clock(oxikube_editor::view::VALIDATION_DEBOUNCE);
    f.vcx.run_until_parked();
    f.press_apply();
    let notice = f.notice(&editor);
    let dialog = f
        .modal::<ApplyPreviewDialog>()
        .unwrap_or_else(|| panic!("the preview; the notice says: {notice}"));
    let title = f.vcx.read(|cx| dialog.read(cx).title().to_string());
    assert_eq!(title, "Create Secret shop/app-creds?");
    let shown = f.vcx.read(|cx| format!("{:?}", dialog.read(cx).diff()));
    assert!(
        !shown.contains(PASSWORD) && !shown.contains(PASSWORD_B64),
        "{shown}"
    );
    f.confirm(&dialog);

    let stored = f
        .resources
        .objects()
        .into_iter()
        .find(|o| o.name() == "app-creds")
        .expect("created")
        .to_value();
    assert_eq!(stored["data"]["password"], PASSWORD_B64);
    assert_eq!(stored["data"]["user"], "YWRtaW4=");
    assert_eq!(stored["type"], "Opaque");
}

#[gpui::test]
fn a_registry_secret_is_made_with_an_auth_string(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.select_kind(SecretKind::Docker, cx);
        dialog.fill_identity("pull", "shop", window, cx);
        dialog.fill_registry("ghcr.io", "octo", "tok3n", "o@example.com", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    let editor = f.only_editor();
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    f.confirm(&dialog);
    let stored = f
        .resources
        .objects()
        .into_iter()
        .find(|o| o.name() == "pull")
        .expect("created")
        .to_value();
    assert_eq!(stored["type"], "kubernetes.io/dockerconfigjson");
    let config = stored["data"][".dockerconfigjson"].as_str().unwrap();
    use base64::Engine as _;
    let config = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(config)
            .unwrap(),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&config).unwrap();
    assert_eq!(parsed["auths"]["ghcr.io"]["auth"], "b2N0bzp0b2szbg==");
    drop(editor);
}

#[gpui::test]
fn a_problem_is_marked_on_its_own_row_when_an_empty_row_comes_first(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.fill_identity("app", "default", window, cx);
        dialog.fill_row(0, "", "", window, cx);
        dialog.fill_row(1, "bad key!", "x", window, cx);
        dialog.fill_row(2, "ok", "", window, cx);
        dialog.fill_row(3, "twice", "1", window, cx);
        dialog.fill_row(4, "twice", "2", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    let fields: Vec<_> = f
        .vcx
        .read(|cx| dialog.read(cx).problems().iter().map(|p| p.field).collect());
    assert_eq!(
        fields,
        [
            oxikube_app::NewSecretField::Key(1),
            oxikube_app::NewSecretField::Value(2),
            oxikube_app::NewSecretField::Key(4),
        ],
        "rows of the dialog, the empty one counted"
    );
    let (blank, bad) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (
            dialog
                .problem(oxikube_app::NewSecretField::Key(0))
                .is_some(),
            dialog
                .problem(oxikube_app::NewSecretField::Key(1))
                .is_some(),
        )
    });
    assert!(!blank && bad);
}

#[gpui::test]
fn a_value_with_control_characters_is_refused_not_stored_as_a_placeholder(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dialog = f.create_dialog();
    dialog.update_in(&mut f.vcx, |dialog, window, cx| {
        dialog.fill_identity("app", "default", window, cx);
        dialog.fill_row(0, "token", "abc\u{1b}[0mdef", window, cx);
        dialog.submit(window, cx);
    });
    f.vcx.run_until_parked();
    let fields: Vec<_> = f
        .vcx
        .read(|cx| dialog.read(cx).problems().iter().map(|p| p.field).collect());
    assert_eq!(fields, [oxikube_app::NewSecretField::Value(0)]);
    assert!(f.modal::<CreateSecretDialog>().is_some(), "still open");
    assert!(f.vcx.read(|cx| {
        f.workspace
            .read(cx)
            .items_of_type::<ManifestEditor>()
            .is_empty()
    }));
}
