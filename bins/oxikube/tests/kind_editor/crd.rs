//! Scenario 4: an instance of the sample CRD checked against the cluster's own schema (E10-S01's
//! `SchemaPort` over the live `/openapi/v3`), and the server's dry run of a bad one mapped onto
//! the same fields.

use std::ops::Range;

use gpui::TestAppContext;
use oxikube_app::{ApplyError, ApplyRequest};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::Gvk;
use oxikube_editor::apply::{SERVER_CODE, place_causes};
use oxikube_editor::view::NoticeLevel;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_ui::editor::{DiagnosticLevel, EditorDiagnostic};

use crate::kind_common::editor::{EditorSession, dynamic_api};
use crate::kind_common::on_cluster;

fn widget_gvk() -> Gvk {
    Gvk::new("test.oxikube.dev", "v1", "Widget")
}

fn widget(namespace: &str, name: &str, spec: &str) -> String {
    format!(
        "apiVersion: test.oxikube.dev/v1\nkind: Widget\nmetadata:\n  name: {name}\n  namespace: {namespace}\nspec:\n{spec}"
    )
}

/// The line `range` starts on.
fn line_of(text: &str, range: &Range<usize>) -> usize {
    text[..range.start].matches('\n').count()
}

/// The diagnostic with `code` whose underlined text is `marked`.
fn marked<'a>(
    diagnostics: &'a [EditorDiagnostic],
    text: &str,
    code: &str,
    marked: &str,
) -> &'a EditorDiagnostic {
    diagnostics
        .iter()
        .find(|d| d.code.as_deref() == Some(code) && text.get(d.range.clone()) == Some(marked))
        .unwrap_or_else(|| panic!("a `{code}` diagnostic on `{marked}`: {diagnostics:#?}"))
}

#[gpui::test]
fn crd_instances_validate_against_the_live_schema_and_the_server_names_the_same_fields(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let ns = namespace.name().to_owned();
    let mut s = EditorSession::start(cx, &context, &ns);

    // A good instance: no diagnostic from the CRD's schema.
    let good = widget(&ns, "good", "  size: small\n  replicas: 2\n");
    let editor = s.new_manifest(&good);
    s.wait_checked(&editor, &[widget_gvk()]);
    assert_eq!(s.diagnostics(&editor), [], "{good}");

    // A bad one: an unknown field (a warning on its key) and a wrong type (an error on the value).
    let bad = widget(
        &ns,
        "bad",
        "  size: small\n  replicas: three\n  colour: red\n",
    );
    s.set_text(&editor, &bad);
    s.wait_checked(&editor, &[widget_gvk()]);
    let client = s.diagnostics(&editor);
    let unknown = marked(&client, &bad, "unknown-field", "colour").clone();
    assert_eq!(unknown.level, DiagnosticLevel::Warning);
    let wrong_type = marked(&client, &bad, "type-mismatch", "three").clone();
    assert_eq!(wrong_type.level, DiagnosticLevel::Error);

    // The editor does not send a manifest with errors.
    s.apply(&editor);
    let notice = s.settled_notice(&editor, "the refusal");
    assert_eq!(notice.level, NoticeLevel::Error, "{notice:?}");
    assert!(notice.text.contains("Fix the 1 error"), "{notice:?}");

    // The server's dry run of each bad variant (through the app's own apply pipeline and guard)
    // names a field the editor already marked, and lands on it: the unknown key exactly, the
    // wrong type on its entry's line.
    let wrong_type_only = widget(&ns, "bad", "  size: small\n  replicas: three\n");
    let variants = [
        (bad.clone(), unknown.range, "colour"),
        // The same text up to the `replicas` line, so the editor's span holds for it too.
        (wrong_type_only, wrong_type.range, "replicas"),
    ];
    for (text, client, key) in variants {
        let service = s.apply_service();
        let request =
            ApplyRequest::new(s.cluster.clone(), text.clone()).in_namespace(Some(ns.clone()));
        let answer = s.on_kube("the server's dry run", async move {
            service.preview(request).await
        });
        let Err(ApplyError::Invalid { causes, message }) = answer else {
            panic!("the server rejects the manifest: {answer:?}");
        };
        let placed = place_causes(&text, &causes);
        assert!(placed.unplaced.is_empty(), "{message}: {placed:?}");
        // The server may name one bad field or all of them, whichever its schema walk meets
        // first: the one the editor marked is among them.
        let server = placed
            .diagnostics
            .iter()
            .find(|d| text.get(d.range.clone()) == Some(key))
            .unwrap_or_else(|| panic!("a cause on `{key}`: {message}: {causes:?}"));
        assert_eq!(server.code.as_deref(), Some(SERVER_CODE));
        assert_eq!(
            line_of(&text, &server.range),
            line_of(&text, &client),
            "the server's field is the one the editor marked: {causes:?}"
        );
    }

    // Nothing was created, and the server's refusals are audited as failed dry runs.
    let widgets = on_cluster(&context, async |client| {
        dynamic_api(client, &ns, ("test.oxikube.dev", "v1", "Widget", "widgets"))
            .list(&kube::api::ListParams::default())
            .await
            .expect("list widgets")
            .items
            .len()
    });
    assert_eq!(widgets, 0);
    let records = s.apply_audit("bad", 2);
    assert!(
        records
            .iter()
            .all(|r| r.dry_run && r.outcome == AuditOutcome::Failed),
        "{records:?}"
    );
}
