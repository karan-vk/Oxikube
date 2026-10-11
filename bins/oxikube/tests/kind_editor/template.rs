//! Scenario 1 and the read-only guard: a ConfigMap from its template, blocked while the session
//! is read-only, then applied, read back and audited.

use gpui::TestAppContext;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::Gvk;
use oxikube_editor::view::NoticeLevel;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};

use crate::kind_common::editor::{EditorSession, configmaps};
use crate::kind_common::{on_cluster, wait_value};

/// The name the ConfigMap template gives its object.
const NAME: &str = "example";

fn configmap_exists(context: &str, namespace: &str) -> bool {
    on_cluster(context, async |client| {
        configmaps(client, namespace)
            .get_opt(NAME)
            .await
            .expect("get")
            .is_some()
    })
}

#[gpui::test]
fn a_configmap_from_its_template_is_refused_read_only_then_applied_and_audited(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let ns = namespace.name().to_owned();
    let mut s = EditorSession::start(cx, &context, &ns);

    let editor = s.create("ConfigMap");
    s.wait_checked(&editor, &[Gvk::new("", "v1", "ConfigMap")]);
    let text = s.text(&editor);
    assert!(
        text.contains(&format!("namespace: {ns}")),
        "the template is filled with the selected namespace:\n{text}"
    );

    // Read-only: the guard refuses the dry run itself; nothing reaches the cluster.
    s.set_read_only(true);
    s.apply(&editor);
    let notice = s.settled_notice(&editor, "the read-only refusal");
    assert_eq!(notice.level, NoticeLevel::Error, "{notice:?}");
    assert!(notice.text.contains("read-only"), "{notice:?}");
    assert!(!configmap_exists(&context, &ns), "nothing was created");
    let denied = s.apply_audit(NAME, 1);
    assert!(
        denied.iter().all(|r| r.outcome == AuditOutcome::Denied),
        "{denied:?}"
    );

    // Writable again: dry run, preview, server-side apply.
    s.set_read_only(false);
    s.apply(&editor);
    let dialog = s.single_preview("the dry run");
    let title = s.vcx.update(|_, cx| dialog.read(cx).title().to_string());
    assert_eq!(title, format!("Create ConfigMap {ns}/{NAME}?"));
    s.confirm(&editor, "the dry run");

    let found = wait_value(&mut s.vcx, "the ConfigMap to exist", |_| {
        on_cluster(&context, async |client| {
            configmaps(client, &ns).get_opt(NAME).await.unwrap()
        })
    });
    assert_eq!(found.data["data"]["key"], "value");
    let managers: Vec<_> = found
        .metadata
        .managed_fields
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| Some((f.manager?, f.operation?)))
        .collect();
    assert!(
        managers.contains(&("oxikube".to_owned(), "Apply".to_owned())),
        "server-side applied under `oxikube`: {managers:?}"
    );

    // Every attempt is in the audit log: the refusal, the dry run and the apply, by name and
    // field manager, never the manifest.
    let records = s.apply_audit(NAME, 3);
    let outcomes: Vec<_> = records.iter().map(|r| (r.dry_run, r.outcome)).collect();
    assert!(
        outcomes.contains(&(true, AuditOutcome::Succeeded)),
        "{outcomes:?}"
    );
    assert!(
        outcomes.contains(&(false, AuditOutcome::Succeeded)),
        "{outcomes:?}"
    );
    for record in &records {
        assert_eq!(record.target.namespace.as_deref(), Some(ns.as_str()));
        let detail = record.detail.as_deref().unwrap_or_default();
        assert!(detail.contains("manager=oxikube"), "{record:?}");
        assert!(!detail.contains("value"), "no manifest content: {record:?}");
    }
}
