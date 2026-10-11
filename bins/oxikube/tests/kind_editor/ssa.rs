//! Scenarios 2 and 3: a live Deployment edited and applied with server-side apply, then the
//! conflict with another field manager (`kubectl`) and Force.

use gpui::{Entity, TestAppContext};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_editor::view::ManifestEditor;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};

use crate::kind_common::editor::EditorSession;
use crate::kind_common::wait;
use crate::objects::{
    SETUP_MANAGER, create_deployment, deployment, kubectl_apply_replicas, owners,
    with_spec_replicas,
};

const REPLICAS: &[&str] = &["spec", "replicas"];

fn deployment_gvk() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

/// "Edit" on the Deployment `name` of the session's namespace, and its text checked against the
/// cluster's schema.
fn edit(s: &mut EditorSession, namespace: &str, name: &str) -> Entity<ManifestEditor> {
    let target = ResourceRef::namespaced(s.cluster.clone(), deployment_gvk(), namespace, name);
    let editor = s.edit(target);
    s.wait_checked(&editor, &[deployment_gvk()]);
    editor
}

/// Changes the editor's `spec.replicas` to `replicas` and waits for the checks of the new text.
fn set_replicas(s: &mut EditorSession, editor: &Entity<ManifestEditor>, replicas: u32) {
    let text = s.text(editor);
    s.set_text(editor, &with_spec_replicas(&text, replicas));
    s.wait_checked(editor, &[deployment_gvk()]);
    let problems = s.vcx.update(|_, cx| editor.read(cx).model().problems());
    assert_eq!(problems.errors, 0, "{}", s.text(editor));
}

/// Waits until the live Deployment has `replicas` in its spec.
fn wait_replicas(s: &mut EditorSession, context: &str, namespace: &str, name: &str, replicas: u64) {
    wait(&mut s.vcx, &format!("spec.replicas = {replicas}"), |_| {
        deployment(context, namespace, name)["spec"]["replicas"].as_u64() == Some(replicas)
    });
}

#[gpui::test]
fn an_edited_deployment_is_server_side_applied_and_oxikube_owns_the_replicas(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let ns = namespace.name().to_owned();
    create_deployment(&context, &ns, "web");
    let mut s = EditorSession::start(cx, &context, &ns);

    // The editor renders the live object as `kubectl get -o yaml` would, `managedFields` hidden.
    let editor = edit(&mut s, &ns, "web");
    let text = s.text(&editor);
    assert!(text.contains("kind: Deployment"), "{text}");
    assert!(
        !text.contains("managedFields"),
        "hidden by default:\n{text}"
    );
    assert!(
        owners(&deployment(&context, &ns, "web"), REPLICAS).is_empty(),
        "a defaulted field has no owner yet"
    );

    set_replicas(&mut s, &editor, 3);
    s.apply(&editor);
    let dialog = s.single_preview("the dry run of the edit");
    let (creates, title) = s.vcx.update(|_, cx| {
        let dialog = dialog.read(cx);
        (dialog.preview().creates(), dialog.title().to_string())
    });
    assert!(!creates, "an update of the live object");
    assert!(title.contains(&format!("{ns}/web")), "{title}");
    s.confirm(&editor, "the dry run of the edit");

    wait_replicas(&mut s, &context, &ns, "web", 3);
    let live = deployment(&context, &ns, "web");
    assert_eq!(
        owners(&live, REPLICAS),
        [("oxikube".to_owned(), "Apply".to_owned())],
        "{}",
        live["metadata"]["managedFields"]
    );
    // The setup's own fields are shared, not taken: same values, no conflict.
    assert!(
        owners(&live, &["spec", "template"])
            .iter()
            .any(|(manager, _)| manager == SETUP_MANAGER),
        "{}",
        live["metadata"]["managedFields"]
    );
    let records = s.apply_audit("web", 2);
    assert!(
        records
            .iter()
            .any(|r| !r.dry_run && r.outcome == AuditOutcome::Succeeded),
        "{records:?}"
    );
}

#[gpui::test]
fn a_conflict_with_kubectl_lists_the_manager_and_force_moves_the_field(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let ns = namespace.name().to_owned();
    create_deployment(&context, &ns, "api");
    kubectl_apply_replicas(&context, &ns, "api", 2);
    assert_eq!(
        owners(&deployment(&context, &ns, "api"), REPLICAS),
        [("kubectl".to_owned(), "Apply".to_owned())]
    );
    let mut s = EditorSession::start(cx, &context, &ns);

    let editor = edit(&mut s, &ns, "api");
    assert!(s.text(&editor).contains("\n  replicas: 2\n"));
    set_replicas(&mut s, &editor, 4);
    s.apply(&editor);

    // The dry run already meets the conflict: the list names kubectl's field, nothing is written.
    let dialog = s.conflicts("the conflict list");
    let (managers, fields, times) = s.vcx.update(|_, cx| {
        let dialog = dialog.read(cx);
        let conflicts = dialog.conflicts();
        (
            dialog
                .managers()
                .iter()
                .map(|m| (*m).to_owned())
                .collect::<Vec<_>>(),
            conflicts
                .iter()
                .map(|c| c.field.clone())
                .collect::<Vec<_>>(),
            conflicts.iter().all(|c| c.applied_at.is_some()),
        )
    });
    assert_eq!(managers, ["kubectl"]);
    assert_eq!(fields, [".spec.replicas"]);
    assert!(times, "when kubectl last wrote the object is shown");
    assert_eq!(
        deployment(&context, &ns, "api")["spec"]["replicas"].as_u64(),
        Some(2),
        "nothing was written"
    );

    // Force: previewed again with force, then applied; the field moves to oxikube.
    dialog.update_in(&mut s.vcx, |dialog, window, cx| dialog.force(window, cx));
    s.confirm(&editor, "the forced dry run");
    wait_replicas(&mut s, &context, &ns, "api", 4);
    let live = deployment(&context, &ns, "api");
    assert_eq!(
        owners(&live, REPLICAS),
        [("oxikube".to_owned(), "Apply".to_owned())],
        "{}",
        live["metadata"]["managedFields"]
    );
    let records = s.apply_audit("api", 3);
    assert!(
        records.iter().any(|r| {
            !r.dry_run
                && r.outcome == AuditOutcome::Succeeded
                && r.detail
                    .as_deref()
                    .is_some_and(|d| d.contains("force=true"))
        }),
        "{records:?}"
    );
}
