//! Create from template and multi-document apply (E10-S09) in a workspace, end to end over the
//! real `ApplyService`, `TemplateService`, `CommandBus` and `MutationGuard`, with the testkit's
//! fake cluster and filesystem on the deterministic runtime:
//!
//! * `resource::Create` opens the "Create resource" picker; choosing Deployment opens a tab with
//!   the cluster's selected namespace in it and no live object; `All` namespaces say `default`
//!   in the notice bar; a user template overrides the embedded one and a broken one is skipped;
//! * a created ConfigMap goes through the dry-run preview and is applied;
//! * a buffer of several documents is dry-run together, previewed once, applied in order, and
//!   each document's result is in the panel under the buffer; a failure in the middle does not
//!   stop the rest; a read-only cluster refuses all of them.

#[path = "apply_common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::*;
use gpui::{Entity, TestAppContext};
use oxikube_app::{DocumentStatus, TemplateService};
use oxikube_domain::OxiError;
use oxikube_domain::Resource;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::diff::SyncState;
use oxikube_editor::templates::{TemplatePicker, embedded};
use oxikube_editor::view::{
    ApplyPreviewDialog, ApplyStage, BatchPreviewDialog, EditorRequest, ManifestEditor,
};
use oxikube_testkit::{FakeFsPort, ScriptedFeed, TICK};
use oxikube_ui::editor::EditorApi as _;

const DIR: &str = "/home/me/.config/oxikube/templates";

fn service(fs: &Arc<FakeFsPort>) -> TemplateService {
    TemplateService::new(embedded(), fs.clone(), PathBuf::from(DIR))
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, Arc<FakeFsPort>) {
    let fs = Arc::new(FakeFsPort::new());
    let f = setup_with(cx, false, Some(service(&fs)));
    (f, fs)
}

impl Fixture {
    fn create(&mut self, template: Option<&str>) {
        self.sink
            .send(EditorRequest::Create {
                cluster: None,
                template: template.map(str::to_owned),
            })
            .unwrap();
        self.vcx.run_until_parked();
    }

    fn select_namespaces(&mut self, selection: NamespaceSelection) {
        self.sessions
            .set_namespace_selection(&cluster(), selection)
            .expect("a connected session");
    }

    fn editors(&mut self) -> Vec<Entity<ManifestEditor>> {
        self.vcx
            .read(|cx| self.workspace.read(cx).items_of_type::<ManifestEditor>())
    }

    fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).text().to_string())
        })
    }

    fn title(&mut self, editor: &Entity<ManifestEditor>) -> String {
        self.vcx.read(|cx| editor.read(cx).title().to_string())
    }

    fn toasts(&mut self) -> Vec<String> {
        self.vcx.read(|cx| {
            self.workspace
                .read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .into_iter()
                .map(|t| t.message.to_string())
                .collect()
        })
    }

    /// Chooses `name` in the open picker, as Enter on it does.
    fn choose(&mut self, picker: &Entity<TemplatePicker>, name: &str) {
        picker.update_in(&mut self.vcx, |picker, window, cx| {
            picker.set_query(name, window, cx);
        });
        self.vcx.run_until_parked();
        picker.update_in(&mut self.vcx, |picker, window, cx| {
            use oxikube_palette::PickerDelegate as _;
            picker.delegate.confirm(false, window, cx);
        });
        self.vcx.run_until_parked();
    }

    fn picker_names(&mut self, picker: &Entity<TemplatePicker>) -> Vec<String> {
        self.vcx.read(|cx| {
            picker
                .read(cx)
                .delegate
                .listed()
                .iter()
                .map(|t| t.name.to_string())
                .collect()
        })
    }
}

#[gpui::test]
fn create_resource_opens_the_picker_and_choosing_a_deployment_fills_in_the_namespace(
    cx: &mut TestAppContext,
) {
    let (mut f, _fs) = fixture(cx);
    f.select_namespaces(NamespaceSelection::single("shop"));
    f.create(None);
    let picker = f.modal::<TemplatePicker>().expect("the picker opened");
    let names = f.picker_names(&picker);
    assert_eq!(names.len(), embedded().len());
    assert_eq!(names[0], "Namespace");

    // Searchable by name (and, for the user's own templates, by kind).
    picker.update_in(&mut f.vcx, |picker, window, cx| {
        picker.set_query("statefu", window, cx);
    });
    f.vcx.run_until_parked();
    assert_eq!(f.picker_names(&picker)[0], "StatefulSet");

    f.choose(&picker, "Deployment");
    assert!(f.modal::<TemplatePicker>().is_none(), "the picker closed");
    let [editor] = f.editors().try_into().expect("one editor opened");
    assert_eq!(f.title(&editor), "New Deployment");
    let text = f.text(&editor);
    assert!(
        text.starts_with("apiVersion: apps/v1\nkind: Deployment\n"),
        "{text}"
    );
    assert!(text.contains("namespace: shop"), "{text}");
    assert!(!text.contains("namespace: default"), "{text}");
    // No live object: not bound to one, not dirty, nothing to say about the namespace.
    f.vcx.read(|cx| {
        let editor = editor.read(cx);
        assert!(editor.binding().is_none());
        assert!(editor.can_apply());
        assert!(!editor.is_dirty(cx));
        assert!(editor.notice().is_none());
    });
}

#[gpui::test]
fn all_namespaces_default_to_default_and_the_notice_says_so(cx: &mut TestAppContext) {
    let (mut f, _fs) = fixture(cx);
    f.select_namespaces(NamespaceSelection::All);
    f.create(Some("configmap"));
    assert!(
        f.modal::<TemplatePicker>().is_none(),
        "a named template opens at once"
    );
    let [editor] = f.editors().try_into().expect("one editor");
    assert!(f.text(&editor).contains("namespace: default"));
    assert_eq!(
        f.notice(&editor),
        "All namespaces are selected: the namespace is set to default"
    );
}

#[gpui::test]
fn an_unknown_template_name_says_so_and_still_offers_the_picker(cx: &mut TestAppContext) {
    let (mut f, _fs) = fixture(cx);
    f.create(Some("no-such-template"));
    assert!(f.editors().is_empty());
    assert!(f.modal::<TemplatePicker>().is_some());
    assert_eq!(f.toasts(), ["There is no template called no-such-template"]);
}

#[gpui::test]
fn user_templates_join_the_picker_override_embedded_ones_and_broken_ones_are_skipped(
    cx: &mut TestAppContext,
) {
    let (mut f, fs) = fixture(cx);
    fs.insert(
        format!("{DIR}/mine.yaml"),
        "# name: ConfigMap\n# description: Our settings\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: ours\n  namespace: default\ndata:\n  team: platform\n",
    );
    fs.insert(
        format!("{DIR}/web.yaml"),
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: default\n",
    );
    fs.insert(format!("{DIR}/broken.yaml"), "a: [unclosed\n");
    f.create(None);
    let picker = f.modal::<TemplatePicker>().expect("the picker");
    let names = f.picker_names(&picker);
    assert_eq!(names.len(), embedded().len() + 1, "one new, one override");
    assert!(names.contains(&"web".to_owned()));
    // The user's `web` is found by its kind too.
    picker.update_in(&mut f.vcx, |picker, window, cx| {
        picker.set_query("deployment", window, cx)
    });
    f.vcx.run_until_parked();
    assert!(f.picker_names(&picker).contains(&"web".to_owned()));
    let skipped = f.vcx.read(|cx| {
        picker
            .read(cx)
            .delegate
            .catalog()
            .skipped()
            .iter()
            .map(|s| s.file.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(skipped, ["broken.yaml"]);

    f.choose(&picker, "ConfigMap");
    let [editor] = f.editors().try_into().expect("one editor");
    assert!(
        f.text(&editor).contains("team: platform"),
        "the user's replaces the embedded one"
    );
}

#[gpui::test]
fn a_created_configmap_is_dry_run_previewed_and_applied(cx: &mut TestAppContext) {
    let (mut f, _fs) = fixture(cx);
    f.select_namespaces(NamespaceSelection::single("shop"));
    f.create(Some("ConfigMap"));
    let [editor] = f.editors().try_into().expect("one editor");
    f.press_apply();
    let dialog = f.modal::<ApplyPreviewDialog>().expect("the preview");
    let title = f.vcx.read(|cx| dialog.read(cx).title().to_string());
    assert_eq!(title, "Create ConfigMap shop/example?");
    assert!(
        f.vcx.read(|cx| dialog.read(cx).preview().creates()),
        "no live object"
    );
    f.confirm(&dialog);
    assert_eq!(f.resources.objects().len(), 1);
    assert_eq!(f.resources.objects()[0].namespace(), Some("shop"));
    assert_eq!(f.notice(&editor), "Applied ConfigMap shop/example");
}

const THREE: &str = "\
apiVersion: v1
kind: Namespace
metadata:
  name: shop
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: a
  namespace: shop
data:
  mode: blue
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: b
  namespace: shop
data:
  mode: green
";

/// Three ConfigMaps that need nothing from each other.
fn independent() -> String {
    ["a", "b", "c"]
        .map(|name| {
            format!("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {name}\ndata:\n  k: v\n")
        })
        .join("---\n")
}

fn names_in_store(f: &Fixture) -> Vec<String> {
    f.resources
        .objects()
        .iter()
        .map(|o| o.name().to_owned())
        .collect()
}

#[gpui::test]
fn a_buffer_of_three_documents_is_previewed_once_and_applied_in_order(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(THREE);
    f.press_apply();

    let dialog = f
        .modal::<BatchPreviewDialog>()
        .expect("one combined preview");
    assert!(f.modal::<ApplyPreviewDialog>().is_none());
    assert_eq!(
        f.writes(),
        [("patch", true), ("patch", true), ("patch", true)]
    );
    let (title, description, documents) = f.vcx.read(|cx| {
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog.description(),
            dialog.preview().documents.len(),
        )
    });
    assert_eq!(title, "Apply 3 documents?");
    assert!(description.contains("3 to create"), "{description}");
    assert!(description.contains("kubectl apply"), "{description}");
    assert_eq!(documents, 3);
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).apply_stage()),
        ApplyStage::Reviewing
    );

    dialog.update_in(&mut f.vcx, |dialog, window, cx| dialog.confirm(window, cx));
    f.vcx.run_until_parked();
    assert!(f.modal::<BatchPreviewDialog>().is_none());
    assert_eq!(names_in_store(&f), ["shop", "a", "b"], "in buffer order");

    let statuses: Vec<String> = f.vcx.read(|cx| {
        editor
            .read(cx)
            .apply_results()
            .expect("the results panel")
            .documents
            .iter()
            .map(|d| format!("{} {}", d.status.word(), d.label))
            .collect()
    });
    assert_eq!(
        statuses,
        [
            "created Namespace shop",
            "created ConfigMap shop/a",
            "created ConfigMap shop/b"
        ]
    );
    assert_eq!(f.notice(&editor), "3 applied");
    assert_eq!(f.toasts(), ["3 applied"]);
    assert!(!f.vcx.read(|cx| editor.read(cx).is_dirty(cx)), "clean");
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
    // Every document was audited on its own: three dry runs, three applies.
    let audit = f.state.audit_log();
    let applied = audit
        .iter()
        .filter(|r| !r.dry_run && r.outcome == AuditOutcome::Succeeded)
        .count();
    assert_eq!(applied, 3);
}

#[gpui::test]
fn a_failure_in_the_middle_is_reported_and_the_rest_still_apply(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(&independent());
    // The dry runs answer in document order: the second one is refused.
    f.resources.script().patch.push_ok(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "a", "namespace": "default"}
        }))
        .unwrap(),
    );
    f.resources
        .script()
        .patch
        .push_err(OxiError::validation("ConfigMap \"b\" is invalid"));
    f.press_apply();
    let dialog = f.modal::<BatchPreviewDialog>().expect("the preview");
    // The refused document is the one shown first.
    assert_eq!(f.vcx.read(|cx| dialog.read(cx).selected()), 1);
    dialog.update_in(&mut f.vcx, |dialog, window, cx| dialog.confirm(window, cx));
    f.vcx.run_until_parked();

    assert_eq!(
        names_in_store(&f),
        ["a", "c"],
        "b was refused, not sent; c was"
    );
    let (words, summary) = f.vcx.read(|cx| {
        let report = editor.read(cx).apply_results().unwrap();
        (
            report
                .documents
                .iter()
                .map(|d| d.status.word())
                .collect::<Vec<_>>(),
            report.summary(),
        )
    });
    assert_eq!(words, ["created", "failed", "created"]);
    assert_eq!(summary, "2 applied, 1 failed");
    assert_eq!(f.notice(&editor), "2 applied, 1 failed");
    assert!(
        f.vcx.read(|cx| editor.read(cx).is_dirty(cx)),
        "not clean while a document failed"
    );
    let failed = f.vcx.read(|cx| {
        matches!(
            editor.read(cx).apply_results().unwrap().documents[1].status,
            DocumentStatus::Failed(_)
        )
    });
    assert!(failed);
}

#[gpui::test]
fn a_row_of_the_results_selects_its_document_in_the_buffer(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(THREE);
    f.press_apply();
    let dialog = f.modal::<BatchPreviewDialog>().expect("the preview");
    dialog.update_in(&mut f.vcx, |dialog, window, cx| dialog.confirm(window, cx));
    f.vcx.run_until_parked();

    let shown = editor.update_in(&mut f.vcx, |editor, window, cx| {
        editor.show_document(2, window, cx)
    });
    assert!(shown);
    let (selection, offset) = f.vcx.update(|window, cx| {
        let code = editor.read(cx).editor().clone();
        let selection = code.update(cx, |code, cx| code.api(window, cx).selections()[0].clone());
        (selection, THREE.rfind("---").unwrap())
    });
    assert_eq!(selection.start, offset, "at the third document's `---`");

    // After an edit the ranges mean nothing: the click does nothing.
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| code.api(window, cx).set_text("kind: x\n"));
    });
    let shown = editor.update_in(&mut f.vcx, |editor, window, cx| {
        editor.show_document(1, window, cx)
    });
    assert!(!shown);
    editor.update(&mut f.vcx, |editor, cx| editor.close_apply_results(cx));
    assert!(f.vcx.read(|cx| editor.read(cx).apply_results().is_none()));
}

#[gpui::test]
fn a_document_in_a_new_namespace_waits_for_the_one_that_makes_it(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(THREE);
    // The server cannot rehearse the ConfigMaps of a namespace that does not exist yet.
    f.resources.script().patch.push_ok(
        oxikube_domain::Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "shop"}
        }))
        .unwrap(),
    );
    for _ in 0..2 {
        f.resources
            .script()
            .patch
            .push_err(OxiError::not_found("namespaces \"shop\" not found"));
    }
    f.press_apply();
    let dialog = f.modal::<BatchPreviewDialog>().expect("the preview");
    let counts = f.vcx.read(|cx| dialog.read(cx).preview().counts());
    assert_eq!((counts.create, counts.deferred, counts.failed), (1, 2, 0));
    dialog.update_in(&mut f.vcx, |dialog, window, cx| dialog.confirm(window, cx));
    f.vcx.run_until_parked();
    assert_eq!(names_in_store(&f), ["shop", "a", "b"]);
    assert_eq!(f.notice(&editor), "3 applied");
}

#[gpui::test]
fn a_read_only_cluster_refuses_every_document_before_any_request(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.editor_with(THREE);
    f.press_apply();
    assert!(f.modal::<BatchPreviewDialog>().is_none());
    assert!(f.resources.recorded_calls().is_empty());
    assert_eq!(
        f.notice(&editor),
        "kind-oxikube is read-only: the manifest was not applied"
    );
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
    assert!(
        f.state
            .audit_log()
            .iter()
            .all(|r| r.outcome == AuditOutcome::Denied)
    );
}

#[gpui::test]
fn cancelling_the_combined_preview_sends_nothing_more(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(THREE);
    f.press_apply();
    let dialog = f.modal::<BatchPreviewDialog>().expect("the preview");
    dialog.update(&mut f.vcx, |dialog, cx| dialog.cancel(cx));
    f.vcx.run_until_parked();
    assert!(f.modal::<BatchPreviewDialog>().is_none());
    assert!(f.writes().iter().all(|(_, dry_run)| *dry_run));
    assert_eq!(f.stage(&editor), ApplyStage::Idle);
    assert!(f.resources.objects().is_empty());
}

#[gpui::test]
fn a_buffer_with_an_error_in_one_document_is_refused_before_the_server_sees_any(
    cx: &mut TestAppContext,
) {
    let mut f = setup(cx, false);
    let editor = f.editor_with(&THREE.replace("name: a\n", "name: a\n  labels: [broken\n"));
    f.press_apply();
    assert!(f.modal::<BatchPreviewDialog>().is_none());
    assert!(f.resources.recorded_calls().is_empty());
    assert!(
        f.notice(&editor).starts_with("Fix the "),
        "{}",
        f.notice(&editor)
    );
}

#[gpui::test]
fn one_document_still_takes_the_single_object_path(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.editor_with(CONFIGMAP);
    f.press_apply();
    assert!(f.modal::<ApplyPreviewDialog>().is_some());
    assert!(f.modal::<BatchPreviewDialog>().is_none());
}

#[gpui::test]
fn pasting_several_documents_into_an_empty_tab_just_works(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    // An empty new tab, the paste, a trailing separator and a comment-only document.
    let pasted = format!("{THREE}---\n# nothing here\n");
    let editor = f.editor_with(&pasted);
    f.press_apply();
    let dialog = f.modal::<BatchPreviewDialog>().expect("the preview");
    let documents = f.vcx.read(|cx| dialog.read(cx).preview().documents.len());
    assert_eq!(documents, 3, "the empty document is not one");
    dialog.update_in(&mut f.vcx, |dialog, window, cx| dialog.confirm(window, cx));
    f.vcx.run_until_parked();
    assert_eq!(names_in_store(&f), ["shop", "a", "b"]);
    assert_eq!(f.notice(&editor), "3 applied");
}

fn web(rv: &str, mode: &str) -> Resource {
    Resource::from_json(serde_json::json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "web", "namespace": "shop", "resourceVersion": rv, "uid": "u-1"},
        "data": {"mode": mode}
    }))
    .unwrap()
}

fn other() -> Resource {
    Resource::from_json(serde_json::json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "other", "namespace": "shop", "resourceVersion": "9"},
        "data": {"k": "v"}
    }))
    .unwrap()
}

#[gpui::test]
fn a_batch_that_writes_the_followed_object_is_not_a_change_on_the_server(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    // The cluster holds `web` at version 1; the watch later delivers our own write, version 2.
    ScriptedFeed::new()
        .initial([web("1", "blue")])
        .modify(1, web("2", "green"))
        .install(&f.resources);
    let editor = f.editor_with(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: web\n  namespace: shop\n",
    );
    f.vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-alt-shift-r"
    } else {
        "ctrl-alt-shift-r"
    });
    f.vcx.run_until_parked();
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).live_sync().state()),
        SyncState::Clean
    );

    // The buffer now holds the followed object edited, and a second document.
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    let text = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: web
  namespace: shop
data:
  mode: green
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: other
  namespace: shop
data:
  k: v
";
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| code.api(window, cx).set_text(text));
    });
    f.vcx.run_until_parked();
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).live_sync().state()),
        SyncState::Dirty
    );

    // Two dry runs, then each document's rehearsal and write; the server stores `web` at 2.
    let (w, o) = (web("2", "green"), other());
    for object in [w.clone(), o.clone(), w.clone(), w, o.clone(), o] {
        f.resources.script().patch.push_ok(object);
    }
    f.press_apply();
    let dialog = f
        .modal::<BatchPreviewDialog>()
        .expect("the combined preview");
    dialog.update_in(&mut f.vcx, |dialog, window, cx| dialog.confirm(window, cx));
    f.vcx.run_until_parked();
    assert_eq!(f.notice(&editor), "2 applied");
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).live_sync().state()),
        SyncState::Clean
    );

    // The watch delivers that write: adopted, not a change made by someone else.
    f.resources.clock().advance(TICK);
    f.vcx.run_until_parked();
    f.vcx.executor().advance_clock(TICK);
    f.vcx.run_until_parked();
    assert_eq!(
        f.vcx.read(|cx| editor.read(cx).live_sync().state()),
        SyncState::Clean
    );
    assert!(
        f.vcx.debug_bounds("manifest-banner").is_none(),
        "no changed-on-server banner for our own write"
    );
}
