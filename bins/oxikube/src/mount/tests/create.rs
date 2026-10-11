//! Create from template and multi-document apply in the running app (E10-S09): `resource::Create`
//! is on the bus with its tool stub (it opens an editor, it changes nothing in a cluster), its
//! shipped key and the table's "New" button open the "Create resource" picker in the cluster's
//! tab, choosing a template opens an editor with the cluster's selected namespace in it, and
//! applying it - one document or several - goes through the guarded `resource::Apply`.

use gpui::TestAppContext;
use oxikube_domain::command::{self, CommandId};
use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::templates::TemplatePicker;
use oxikube_editor::view::{ApplyPreviewDialog, BatchPreviewDialog, ManifestEditor};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn create_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-alt-n"
    } else {
        "ctrl-alt-n"
    }
}

fn apply_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    }
}

fn new_manifest_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    }
}

impl App {
    fn picker(&mut self) -> Option<gpui::Entity<TemplatePicker>> {
        let workspace = self.tab_workspace();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<TemplatePicker>()
        })
    }

    fn editors(&mut self) -> Vec<gpui::Entity<ManifestEditor>> {
        let workspace = self.tab_workspace();
        self.vcx
            .update(|_, cx| workspace.read(cx).items_of_type::<ManifestEditor>())
    }

    fn text_of(&mut self, editor: &gpui::Entity<ManifestEditor>) -> String {
        let code = self.vcx.update(|_, cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| {
                use oxikube_ui::editor::EditorApi as _;
                code.api(window, cx).text().to_string()
            })
        })
    }

    fn select_namespace(&mut self, name: &str) {
        let state = self.vcx.update(|_, cx| AppState::global(cx));
        state
            .services()
            .sessions
            .set_namespace_selection(&TestPorts::cluster_id(), NamespaceSelection::single(name))
            .expect("a session");
    }
}

#[gpui::test]
fn resource_create_is_on_the_bus_with_a_tool_stub_and_changes_nothing_in_a_cluster(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(
        bus.owner(CommandId::RESOURCE_CREATE),
        Some("oxikube_editor")
    );
    let tool = bus.tool(CommandId::RESOURCE_CREATE).expect("a tool stub");
    assert_eq!(tool.name.as_str(), "k8s.resource_create");
    // Creating is `resource::Apply`, which is guarded (see `apply.rs`): this command opens the
    // editor on a template and sends nothing.
    let meta = command::lookup(CommandId::RESOURCE_CREATE).unwrap();
    assert!(!meta.mutating);
}

#[gpui::test]
fn the_key_opens_the_picker_and_a_template_opens_with_the_selected_namespace(
    cx: &mut TestAppContext,
) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.select_namespace("kube-system");

    app.press(create_key());
    let picker = app
        .picker()
        .expect("the picker opened in the cluster's tab");
    assert!(
        app.drawn("template-picker-footer"),
        "the user directory is named"
    );
    assert!(
        app.vcx
            .update(|_, cx| picker.read(cx).delegate.listed().len())
            > 20
    );

    app.vcx.simulate_input("configmap");
    app.vcx.run_until_parked();
    let first = app.vcx.update(|_, cx| {
        picker
            .read(cx)
            .delegate
            .selected()
            .map(|t| t.name.to_string())
    });
    assert_eq!(first.as_deref(), Some("ConfigMap"));
    app.press("enter");

    assert!(app.picker().is_none(), "the picker closed");
    let editors = app.editors();
    assert_eq!(editors.len(), 1, "an editor in the cluster's tab");
    let text = app.text_of(&editors[0]);
    assert!(
        text.starts_with("apiVersion: v1\nkind: ConfigMap\n"),
        "{text}"
    );
    assert!(text.contains("namespace: kube-system"), "{text}");
    let title = app
        .vcx
        .update(|_, cx| editors[0].read(cx).title().to_string());
    assert_eq!(title, "New ConfigMap");
    assert!(
        app.vcx
            .update(|_, cx| editors[0].read(cx).binding().is_none())
    );
}

#[gpui::test]
fn a_created_template_is_applied_through_the_guarded_preview(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let resources = ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.press(create_key());
    app.vcx.simulate_input("configmap");
    app.press("enter");
    assert_eq!(app.editors().len(), 1);

    app.press(apply_key());
    let workspace = app.tab_workspace();
    let dialog = app.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<ApplyPreviewDialog>()
    });
    let dialog = dialog.expect("the dry-run preview");
    let title = app.vcx.update(|_, cx| dialog.read(cx).title().to_string());
    assert_eq!(title, "Create ConfigMap default/example?");
    assert!(
        resources.mutating_calls().iter().all(|c| c.is_dry_run()),
        "nothing written before the user confirms"
    );
    app.click("apply-preview-confirm");
    assert!(
        resources
            .objects()
            .iter()
            .any(|o| o.name() == "example" && o.namespace() == Some("default"))
    );
}

#[gpui::test]
fn several_documents_are_previewed_together_and_applied_in_order(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let resources = ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;
    let mut app = App::start(cx, ports);
    app.press("enter");
    app.press(new_manifest_key());
    app.vcx.simulate_input(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: first\n  namespace: default\ndata:\n  a: b\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: second\n  namespace: default\ndata:\n  c: d\n",
    );
    app.vcx
        .executor()
        .advance_clock(oxikube_editor::view::VALIDATION_DEBOUNCE);
    app.vcx.run_until_parked();

    app.press(apply_key());
    let workspace = app.tab_workspace();
    assert!(
        app.vcx
            .update(|_, cx| workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<BatchPreviewDialog>())
            .is_some(),
        "one combined preview"
    );
    assert!(app.drawn("batch-preview-documents"));
    assert_eq!(
        resources.mutating_calls().len(),
        2,
        "a dry run per document"
    );

    app.click("batch-preview-confirm");
    let names: Vec<String> = resources
        .objects()
        .iter()
        .map(|o| o.name().to_owned())
        .collect();
    assert_eq!(names, ["first", "second"], "in buffer order");
    assert!(
        app.drawn("manifest-apply-results"),
        "the results panel is under the buffer"
    );
    assert!(app.drawn("manifest-apply-result-1"));
}

#[gpui::test]
fn the_new_button_of_a_resource_table_opens_the_picker(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    app.click("resource-table-create");
    assert!(app.picker().is_some(), "the Create resource picker");
}

#[gpui::test]
fn the_palette_lists_create_resource_and_confirming_it_opens_the_picker(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    let palette = if cfg!(target_os = "macos") {
        "cmd-shift-p"
    } else {
        "ctrl-shift-p"
    };
    app.press(palette);
    app.vcx.simulate_input("create resource");
    app.vcx.run_until_parked();
    app.press("enter");
    app.tick();
    assert!(app.picker().is_some(), "the palette ran resource::Create");
}
