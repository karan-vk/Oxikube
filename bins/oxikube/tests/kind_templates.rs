//! Create from template and multi-document apply against a real cluster (E10-S09), through the
//! app: the `resource::Create` command opens a template in the connected cluster's tab, the
//! editor checks it against the cluster's own OpenAPI schemas, and the apply flow dry-runs it on
//! the real API server.
//!
//! * every embedded template (one per core kind, 27 of them) opens clean - no error, no warning
//!   from the cluster's schemas - and its server dry run is accepted, in a namespace of the
//!   test's own that the cluster's selection names;
//! * a ConfigMap made from its template is created in that namespace (the epic's fifth "done
//!   when");
//! * a buffer of several documents - two ConfigMaps and a new namespace with a ConfigMap in it,
//!   which the server cannot rehearse until the namespace exists - is previewed once and applied
//!   in order.
//!
//! `cargo test -p oxikube --features integration --test kind_templates` with
//! `OXIKUBE_TEST_CONTEXT` set (`cargo xtask kind-up`); without it each test returns at once.
//! Every object lives in a namespace the test creates and deletes.
#![cfg(feature = "integration")]

mod kind_common;

use gpui::TestAppContext;
use oxikube_domain::ids::Gvk;
use oxikube_editor::templates::embedded;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};

use kind_common::editor::{EditorSession as Session, configmaps, gvks_in};
use kind_common::{on_cluster, wait_value};

/// Removes a namespace a buffer under test created, even when an assertion fails first (the
/// testkit's `TestNamespace` only knows its own).
struct DeleteNamespaceOnDrop {
    context: String,
    name: String,
}

impl Drop for DeleteNamespaceOnDrop {
    fn drop(&mut self) {
        // Best effort, like the testkit's: never panic in drop.
        let _ = std::process::Command::new("kubectl")
            .args(["--context", &self.context, "delete", "namespace"])
            .args([&self.name, "--ignore-not-found", "--wait=false"])
            .output();
    }
}

#[gpui::test]
fn every_template_opens_clean_against_the_clusters_schemas_and_passes_a_server_dry_run(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let mut s = Session::start(cx, &context, namespace.name());

    for template in embedded() {
        let name = template.name.to_string();
        let editor = s.create(&name);
        let text = s.text(&editor);
        assert!(!text.contains("namespace: default"), "{name}: {text}");
        s.wait_checked(&editor, &gvks_in(&text));
        let problems = s.vcx.update(|_, cx| editor.read(cx).model().problems());
        assert_eq!(
            (problems.errors, problems.warnings),
            (0, 0),
            "{name} against the cluster's own schema:\n{text}"
        );

        s.apply(&editor);
        if template.documents > 1 {
            let dialog = s.batch_preview(&format!("the {name} dry run"));
            let (counts, labels) = s.vcx.update(|_, cx| {
                let preview = dialog.read(cx).preview().clone();
                let labels: Vec<String> = preview
                    .documents
                    .iter()
                    .map(|d| format!("{} {:?}", d.label, d.outcome))
                    .collect();
                (preview.counts(), labels)
            });
            assert_eq!(counts.create, template.documents, "{name}: {labels:#?}");
            dialog.update(&mut s.vcx, |dialog, cx| dialog.cancel(cx));
        } else {
            let dialog = s.single_preview(&format!("the {name} dry run"));
            let creates = s.vcx.update(|_, cx| dialog.read(cx).preview().creates());
            assert!(creates, "{name} is a new object");
            dialog.update(&mut s.vcx, |dialog, cx| dialog.cancel(cx));
        }
        s.vcx.run_until_parked();
    }
    // Nothing was written: only dry runs.
    let leftovers = on_cluster(&context, async |client| {
        configmaps(client, namespace.name())
            .list(&kube::api::ListParams::default())
            .await
            .unwrap()
            .items
            .iter()
            // The cluster adds `kube-root-ca.crt` to every namespace.
            .filter(|cm| cm.metadata.name.as_deref() != Some("kube-root-ca.crt"))
            .count()
    });
    assert_eq!(leftovers, 0);
}

#[gpui::test]
fn a_configmap_made_from_its_template_is_created_in_the_selected_namespace(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    let mut s = Session::start(cx, &context, namespace.name());

    let editor = s.create("ConfigMap");
    s.wait_checked(&editor, &[Gvk::new("", "v1", "ConfigMap")]);
    s.apply(&editor);
    let dialog = s.single_preview("the dry run");
    let title = s.vcx.update(|_, cx| dialog.read(cx).title().to_string());
    assert_eq!(
        title,
        format!("Create ConfigMap {}/example?", namespace.name())
    );
    dialog.update_in(&mut s.vcx, |dialog, window, cx| dialog.confirm(window, cx));

    let name = namespace.name().to_owned();
    let found = wait_value(&mut s.vcx, "the ConfigMap to exist", |_| {
        on_cluster(&context, async |client| {
            configmaps(client, &name).get_opt("example").await.unwrap()
        })
    });
    assert_eq!(found.data["data"]["key"], "value");
    let managed = found
        .metadata
        .managed_fields
        .unwrap_or_default()
        .iter()
        .any(|f| f.manager.as_deref() == Some("oxikube"));
    assert!(
        managed,
        "server-side applied under the field manager `oxikube`"
    );
}

#[gpui::test]
fn a_buffer_of_several_documents_is_applied_in_order_with_a_new_namespace_first(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let namespace = TestNamespace::create(&context).expect("a namespace");
    // A second namespace that the buffer itself creates; removed when the test ends, however
    // it ends.
    let extra = format!("{}-b", namespace.name());
    let _extra_namespace = DeleteNamespaceOnDrop {
        context: context.clone(),
        name: extra.clone(),
    };
    let mut s = Session::start(cx, &context, namespace.name());
    let ns = namespace.name();
    let text = format!(
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {extra}\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: first\n  namespace: {ns}\ndata:\n  k: one\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: second\n  namespace: {extra}\ndata:\n  k: two\n"
    );
    let editor = s.new_manifest(&text);

    s.apply(&editor);
    let dialog = s.batch_preview("the combined dry run");
    let counts = s.vcx.update(|_, cx| dialog.read(cx).preview().counts());
    assert_eq!(
        (counts.create, counts.deferred, counts.failed),
        (2, 1, 0),
        "the ConfigMap in the new namespace waits for it"
    );
    dialog.update_in(&mut s.vcx, |dialog, window, cx| dialog.confirm(window, cx));

    let results = wait_value(&mut s.vcx, "the results", |vcx| {
        vcx.update(|_, cx| {
            editor.read(cx).apply_results().map(|report| {
                report
                    .documents
                    .iter()
                    .map(|d| (d.status.word(), d.message.clone()))
                    .collect::<Vec<_>>()
            })
        })
    });
    assert_eq!(
        results.iter().map(|(word, _)| *word).collect::<Vec<_>>(),
        ["created", "created", "created"],
        "{results:?}"
    );
    let (extra_name, ns_name) = (extra, ns.to_owned());
    let found = on_cluster(&context, async |client| {
        (
            configmaps(client.clone(), &ns_name)
                .get_opt("first")
                .await
                .unwrap()
                .is_some(),
            configmaps(client, &extra_name)
                .get_opt("second")
                .await
                .unwrap()
                .is_some(),
        )
    });
    assert_eq!(found, (true, true));
}
