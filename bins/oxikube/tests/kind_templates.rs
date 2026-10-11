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

use std::time::Duration;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::apply::split_documents;
use oxikube_editor::templates::embedded;
use oxikube_editor::view::{
    ApplyPreviewDialog, BatchPreviewDialog, ManifestEditor, VALIDATION_DEBOUNCE,
};
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::{ClusterTab, Workspace};

use kind_common::{Launched, on_cluster, wait, wait_value};

fn settle(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(VALIDATION_DEBOUNCE);
    vcx.run_until_parked();
}

/// What the tests do in the connected cluster's tab.
struct Session {
    vcx: VisualTestContext,
    inner: Entity<Workspace>,
    cluster: ClusterId,
    _tab: Entity<ClusterTab>,
}

impl Session {
    fn start(cx: &mut TestAppContext, context: &str, namespace: &str) -> Self {
        cx.executor().allow_parking();
        let Launched {
            mut vcx,
            tab,
            cluster,
            ..
        } = kind_common::launch(cx, context);
        let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
        let state = vcx.update(|_, cx| AppState::global(cx));
        state
            .services()
            .sessions
            .set_namespace_selection(&cluster, NamespaceSelection::single(namespace))
            .expect("a session");
        Self {
            vcx,
            inner,
            cluster,
            _tab: tab,
        }
    }

    fn dispatch(&mut self, command: Command) {
        let state = self.vcx.update(|_, cx| AppState::global(cx));
        let bus = state.command_bus().expect("the bus").clone();
        block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")))
            .expect("the command runs");
        self.vcx.run_until_parked();
    }

    fn editors(&mut self) -> Vec<Entity<ManifestEditor>> {
        self.vcx
            .update(|_, cx| self.inner.read(cx).items_of_type::<ManifestEditor>())
    }

    /// Opens the template `name` in the cluster's tab and returns its editor.
    fn create(&mut self, name: &str) -> Entity<ManifestEditor> {
        let before = self.editors().len();
        self.dispatch(Command::ResourceCreate {
            cluster: Some(self.cluster.clone()),
            template: Some(name.to_owned()),
        });
        wait(&mut self.vcx, &format!("the {name} editor"), |vcx| {
            vcx.update(|_, cx| self.inner.read(cx).items_of_type::<ManifestEditor>().len()) > before
        });
        self.vcx.update(|_, cx| {
            self.inner
                .read(cx)
                .active_item(cx)
                .and_then(|item| item.downcast::<ManifestEditor>())
                .expect("the new editor is the active tab")
        })
    }

    fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.update(|_, cx| editor.read(cx).editor().clone());
        self.vcx
            .update(|window, cx| code.update(cx, |c, cx| c.api(window, cx).text().to_string()))
    }

    /// Waits for the cluster's schema of every kind in `editor`'s text, then for the checks.
    fn wait_checked(&mut self, editor: &Entity<ManifestEditor>, kinds: &[Gvk]) {
        for gvk in kinds {
            wait(&mut self.vcx, &format!("the schema of {gvk}"), |vcx| {
                settle(vcx);
                vcx.update(|_, cx| {
                    editor
                        .read(cx)
                        .model()
                        .known()
                        .get(gvk)
                        .is_some_and(Option::is_some)
                })
            });
        }
        settle(&mut self.vcx);
    }

    fn apply(&mut self, editor: &Entity<ManifestEditor>) {
        editor.update_in(&mut self.vcx, |view, window, cx| {
            view.begin_apply(window, cx)
        });
        self.vcx.run_until_parked();
    }

    fn single_preview(&mut self, what: &str) -> Entity<ApplyPreviewDialog> {
        wait_value(&mut self.vcx, what, |vcx| {
            vcx.update(|_, cx| {
                self.inner
                    .read(cx)
                    .modal_layer()
                    .read(cx)
                    .active_modal::<ApplyPreviewDialog>()
            })
        })
    }

    fn batch_preview(&mut self, what: &str) -> Entity<BatchPreviewDialog> {
        wait_value(&mut self.vcx, what, |vcx| {
            vcx.update(|_, cx| {
                self.inner
                    .read(cx)
                    .modal_layer()
                    .read(cx)
                    .active_modal::<BatchPreviewDialog>()
            })
        })
    }
}

/// The kind of each document of `text`.
fn gvks_in(text: &str) -> Vec<Gvk> {
    split_documents(text)
        .iter()
        .filter_map(|document| {
            let field = |key: &str| {
                document
                    .text
                    .lines()
                    .find_map(|line| line.strip_prefix(key)?.strip_prefix(": "))
            };
            Some(Gvk::from_api_version(
                field("apiVersion")?.trim(),
                field("kind")?.trim(),
            ))
        })
        .collect()
}

/// The ConfigMaps API of `namespace`, as the dynamic client sees it.
fn configmaps(client: kube::Client, namespace: &str) -> kube::Api<kube::api::DynamicObject> {
    let resource = kube::api::ApiResource::from_gvk_with_plural(
        &kube::api::GroupVersionKind::gvk("", "v1", "ConfigMap"),
        "configmaps",
    );
    kube::Api::namespaced_with(client, namespace, &resource)
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
    // A second namespace that the buffer itself creates; removed at the end.
    let extra = format!("{}-b", namespace.name());
    let mut s = Session::start(cx, &context, namespace.name());
    let ns = namespace.name();
    let text = format!(
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {extra}\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: first\n  namespace: {ns}\ndata:\n  k: one\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: second\n  namespace: {extra}\ndata:\n  k: two\n"
    );
    s.dispatch(Command::EditorNewManifest {
        cluster: Some(s.cluster.clone()),
    });
    let editor = s.vcx.update(|_, cx| {
        s.inner
            .read(cx)
            .items_of_type::<ManifestEditor>()
            .pop()
            .expect("the new editor")
    });
    let code = s.vcx.update(|_, cx| editor.read(cx).editor().clone());
    s.vcx
        .update(|window, cx| code.update(cx, |c, cx| c.api(window, cx).set_text(&text)));
    s.vcx.run_until_parked();

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
    let (extra_name, ns_name) = (extra.clone(), ns.to_owned());
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

    // Clean up the namespace the buffer made (the test's own is removed on drop).
    let _ = std::process::Command::new("kubectl")
        .args([
            "--context",
            &context,
            "delete",
            "namespace",
            &extra,
            "--wait=false",
        ])
        .output();
    std::thread::sleep(Duration::from_millis(10));
}
