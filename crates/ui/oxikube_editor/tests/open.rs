//! An editor opened from a live object (E10-S06): `resource::Edit` and
//! `resource::ViewYamlInEditor` through the window's `EditorViews` open a tab titled `kind/name`
//! that reads the object fresh from the cluster's reader (a fake on the deterministic runtime);
//! the same object reuses its tab; a Secret opens masked and reveals on request; `managedFields`
//! are hidden and toggle; a read-only cluster opens a read-only tab; nothing of a Secret reaches
//! the saved layout.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use async_trait::async_trait;
use futures::channel::oneshot;
use gpui::{App, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{OxiResult, Resource};
use oxikube_editor::open::{MASKED, OpenMode, ResourceLoad};
use oxikube_editor::view::{
    EditorHost, EditorRequest, EditorViewSink, EditorViews, EditorViewsDeps, ManifestEditor,
    SchemaSource, VALIDATION_DEBOUNCE,
};
use oxikube_ports::{
    ListOptions, ListPage, ResourceReader, SchemaPort, Subresource, WatchFeed, WatchOptions,
};
use oxikube_testkit::fakes::{FakeResourcePort, FakeSchemaPort, ResourceCall};
use oxikube_ui::editor::{DecorationStyle, EditorApi as _};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Item as _, Workspace};
use parking_lot::Mutex;
use serde_json::json;

fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

fn gvk(group: &str, version: &str, kind: &str) -> Gvk {
    Gvk::new(group, version, kind)
}

fn target(gvk: Gvk, name: &str) -> ResourceRef {
    ResourceRef::new(cluster(), gvk, Some("default".into()), name)
}

fn deployment_ref() -> ResourceRef {
    target(gvk("apps", "v1", "Deployment"), "web")
}

fn secret_ref() -> ResourceRef {
    target(gvk("", "v1", "Secret"), "db")
}

fn deployment(replicas: u64) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "web",
            "namespace": "default",
            "managedFields": [{"manager": "kubectl", "operation": "Apply"}]
        },
        "spec": {"replicas": replicas},
        "status": {"availableReplicas": replicas}
    }))
    .expect("fixture Deployment is a valid resource")
}

const PASSWORD: &str = "c2VjcmV0LXBhc3N3b3Jk";

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": "db", "namespace": "default"},
        "data": {"password": PASSWORD, "username": "YWRtaW4="}
    }))
    .expect("fixture Secret is a valid resource")
}

/// The bus, reduced to what the editor does: queue the request.
struct SinkDispatcher {
    sink: EditorViewSink,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl CommandDispatcher for SinkDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = EditorRequest::of(command) {
            self.sink.send(request).expect("the window is open");
        }
    }
}

struct Schemas(Arc<FakeSchemaPort>);

impl SchemaSource for Schemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.0.clone())
    }
}

/// The fake reader whose next `get` waits until the test lets it through (the object is "being
/// read" for as long as the test wants).
struct GatedReader {
    inner: Arc<FakeResourcePort>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl ResourceReader for GatedReader {
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        self.inner.list(kind, namespace, options).await
    }
    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<oxikube_domain::ObjectMeta>> {
        self.inner.list_metadata(kind, namespace, options).await
    }
    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource> {
        let gate = self.gate.lock().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        self.inner.get(kind, namespace, name).await
    }
    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        self.inner.get_opt(kind, namespace, name).await
    }
    async fn watch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        self.inner.watch(kind, namespace, options).await
    }
    async fn get_scale(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<oxikube_ports::Scale> {
        self.inner.get_scale(kind, namespace, name).await
    }
    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<serde_json::Value> {
        self.inner
            .get_subresource(kind, namespace, name, subresource)
            .await
    }
}

struct Host {
    workspace: WeakEntity<Workspace>,
    reader: Arc<GatedReader>,
    schemas: Arc<FakeSchemaPort>,
    read_only: bool,
}

impl EditorHost for Host {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        Some(cluster())
    }
    fn workspace(&self, _: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        self.workspace.upgrade()
    }
    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}
    fn schemas(&self, _: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(Schemas(self.schemas.clone()))
    }
    fn reader(&self, _: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        Some(self.reader.clone())
    }
    fn cluster_read_only(&self, _: &ClusterId) -> bool {
        self.read_only
    }
}

struct Fixture {
    workspace: Entity<Workspace>,
    vcx: VisualTestContext,
    sink: EditorViewSink,
    sent: Rc<RefCell<Vec<Command>>>,
    resources: Arc<FakeResourcePort>,
    reader: Arc<GatedReader>,
    _views: Entity<EditorViews>,
}

fn setup(cx: &mut TestAppContext, read_only: bool) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let resources = Arc::new(FakeResourcePort::new());
    resources.insert(deployment(2));
    resources.insert(secret());
    let sent = Rc::new(RefCell::new(Vec::new()));
    let (sink, requests) = EditorViewSink::channel();
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(SinkDispatcher {
        sink: sink.clone(),
        sent: sent.clone(),
    });
    let reader = Arc::new(GatedReader {
        inner: resources.clone(),
        gate: Mutex::new(None),
    });
    let host = Rc::new(Host {
        workspace: workspace.downgrade(),
        reader: reader.clone(),
        schemas: Arc::new(FakeSchemaPort::new()),
        read_only,
    });
    let views = vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        oxikube_editor::view::install(dispatcher.clone(), cx);
        oxikube_keymap::init_with_text("", oxikube_keymap::KeymapOptions::default(), cx);
        let deps = EditorViewsDeps {
            host,
            window_workspace: workspace.downgrade(),
            dispatcher,
        };
        EditorViews::start(deps, requests, window, cx)
    });
    vcx.run_until_parked();
    Fixture {
        workspace,
        vcx,
        sink,
        sent,
        resources,
        reader,
        _views: views,
    }
}

impl Fixture {
    /// The next `get` waits for the returned sender.
    fn hold_next_get(&self) -> oneshot::Sender<()> {
        let (release, gate) = oneshot::channel();
        *self.reader.gate.lock() = Some(gate);
        release
    }

    fn open(&mut self, target: ResourceRef, mode: OpenMode) {
        self.sink
            .send(EditorRequest::Open { target, mode })
            .expect("the window is open");
        self.vcx.run_until_parked();
    }

    fn editors(&mut self) -> Vec<Entity<ManifestEditor>> {
        self.vcx
            .read(|cx| self.workspace.read(cx).items_of_type::<ManifestEditor>())
    }

    fn only_editor(&mut self) -> Entity<ManifestEditor> {
        let mut editors = self.editors();
        assert_eq!(editors.len(), 1, "one editor tab");
        editors
            .pop()
            .expect("exactly one editor was asserted above")
    }

    fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).text().to_string())
        })
    }

    fn title(&mut self, editor: &Entity<ManifestEditor>) -> String {
        self.vcx
            .read(|cx| editor.read(cx).tab_content(cx).title.to_string())
    }

    fn read_only(&mut self, editor: &Entity<ManifestEditor>) -> bool {
        self.vcx
            .read(|cx| editor.read(cx).editor().read(cx).is_read_only())
    }

    fn dirty(&mut self, editor: &Entity<ManifestEditor>) -> bool {
        self.vcx.read(|cx| editor.read(cx).is_dirty(cx))
    }

    fn problems(&mut self, editor: &Entity<ManifestEditor>) -> (usize, usize) {
        let p = self.vcx.read(|cx| editor.read(cx).model().problems());
        (p.errors, p.warnings)
    }

    fn validate(&mut self) {
        self.vcx.executor().advance_clock(VALIDATION_DEBOUNCE * 2);
        self.vcx.run_until_parked();
    }

    fn gets(&self) -> usize {
        self.resources
            .recorded_calls()
            .iter()
            .filter(|call| matches!(call, ResourceCall::Get { .. }))
            .count()
    }

    fn active_editor(&mut self) -> Option<Entity<ManifestEditor>> {
        self.vcx.read(|cx| {
            self.workspace
                .read(cx)
                .active_item(cx)?
                .downcast::<ManifestEditor>()
        })
    }
}

fn alt_m() -> &'static str {
    "alt-m"
}

fn alt_v() -> &'static str {
    "alt-v"
}

#[gpui::test]
fn edit_opens_an_editable_tab_titled_kind_slash_name(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(deployment_ref(), OpenMode::Edit);
    let editor = f.only_editor();
    assert_eq!(f.title(&editor), "Deployment/web");
    let text = f.text(&editor);
    assert!(text.starts_with("apiVersion: apps/v1\nkind: Deployment\nmetadata:\n"));
    assert!(text.contains("replicas: 2"));
    assert!(!f.read_only(&editor), "Edit opens an editable tab");
    assert!(!f.dirty(&editor), "a freshly opened object is not an edit");
    let binding = f
        .vcx
        .read(|cx| editor.read(cx).binding().map(|b| b.load().clone()));
    assert_eq!(binding, Some(ResourceLoad::Ready));
    assert_eq!(f.gets(), 1, "read with a fresh get");

    f.vcx.simulate_input("# note\n");
    assert!(f.text(&editor).starts_with("# note\n"));
    assert!(f.dirty(&editor));
}

#[gpui::test]
fn the_tab_starts_from_a_fresh_get_not_from_a_cache(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.resources.insert(deployment(7));
    f.open(deployment_ref(), OpenMode::View);
    let editor = f.only_editor();
    assert!(f.text(&editor).contains("replicas: 7"));
    assert_eq!(f.gets(), 1);
}

#[gpui::test]
fn the_same_object_reuses_its_tab(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(deployment_ref(), OpenMode::Edit);
    let first = f.only_editor();
    // Another tab takes the focus.
    f.sink
        .send(EditorRequest::New { cluster: None })
        .expect("open");
    f.vcx.run_until_parked();
    assert_eq!(f.editors().len(), 2);
    assert_ne!(f.active_editor(), Some(first.clone()));

    f.open(deployment_ref(), OpenMode::Edit);
    assert_eq!(f.editors().len(), 2, "no second Deployment/web tab");
    assert_eq!(
        f.active_editor(),
        Some(first),
        "the existing tab is focused"
    );
    assert_eq!(f.gets(), 1, "and not read again");
}

#[gpui::test]
fn view_yaml_opens_read_only_and_edit_unlocks_the_same_tab(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(deployment_ref(), OpenMode::View);
    let editor = f.only_editor();
    assert_eq!(f.title(&editor), "Deployment/web");
    assert!(f.read_only(&editor));
    let before = f.text(&editor);
    f.vcx.simulate_input("typing is refused");
    assert_eq!(f.text(&editor), before, "a view tab rejects typing");

    f.open(deployment_ref(), OpenMode::Edit);
    assert_eq!(f.editors().len(), 1);
    assert!(!f.read_only(&editor), "Edit makes the same tab editable");
}

#[gpui::test]
fn status_is_dimmed_and_the_dimming_is_a_decoration(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(deployment_ref(), OpenMode::Edit);
    let editor = f.only_editor();
    let text = f.text(&editor);
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    let decorations = f
        .vcx
        .update(|window, cx| code.update(cx, |code, cx| code.api(window, cx).decorations()));
    assert_eq!(decorations.len(), 1);
    assert_eq!(decorations[0].style, DecorationStyle::Dimmed);
    assert!(text[decorations[0].range.clone()].starts_with("status:"));
}

#[gpui::test]
fn managed_fields_are_hidden_and_the_key_toggles_them(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(deployment_ref(), OpenMode::View);
    let editor = f.only_editor();
    assert!(!f.text(&editor).contains("managedFields"));
    let hidden = f
        .vcx
        .read(|cx| editor.read(cx).binding().unwrap().hidden_managed_fields());
    assert_eq!(hidden, 1);

    f.vcx.simulate_keystrokes(alt_m());
    f.vcx.run_until_parked();
    assert!(
        f.sent
            .borrow()
            .contains(&Command::EditorToggleManagedFields)
    );
    assert!(f.text(&editor).contains("managedFields:"));
    assert!(!f.dirty(&editor), "a re-render is not an edit");
    assert!(f.read_only(&editor), "and keeps the view tab read-only");

    f.vcx.simulate_keystrokes(alt_m());
    f.vcx.run_until_parked();
    assert!(!f.text(&editor).contains("managedFields"));
}

#[gpui::test]
fn a_secret_opens_masked_and_the_key_reveals_it_in_memory_only(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(secret_ref(), OpenMode::View);
    let editor = f.only_editor();
    assert_eq!(f.title(&editor), "Secret/db");
    let masked = f.text(&editor);
    assert!(!masked.contains(PASSWORD), "{masked}");
    assert!(!masked.contains("YWRtaW4="));
    assert_eq!(masked.matches(MASKED).count(), 2);
    // Each placeholder left in the buffer is an error: it can never be applied as a value.
    f.validate();
    assert_eq!(f.problems(&editor), (2, 0));

    f.vcx.simulate_keystrokes(alt_v());
    f.vcx.run_until_parked();
    assert!(f.sent.borrow().contains(&Command::EditorToggleSecretMask));
    let revealed = f.text(&editor);
    assert!(revealed.contains(&format!("password: {PASSWORD}")));
    assert!(!revealed.contains(MASKED));
    f.validate();
    assert_eq!(
        f.problems(&editor),
        (0, 0),
        "nothing masked, nothing blocked"
    );

    f.vcx.simulate_keystrokes(alt_v());
    f.vcx.run_until_parked();
    assert!(!f.text(&editor).contains(PASSWORD), "masked again");
}

#[gpui::test]
fn the_reveal_does_not_survive_the_tab_and_nothing_reaches_the_layout(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(secret_ref(), OpenMode::View);
    let editor = f.only_editor();
    f.vcx.simulate_keystrokes(alt_v());
    f.vcx.run_until_parked();
    assert!(f.text(&editor).contains(PASSWORD));

    // The saved layout (what the window persists) holds no value and no editor.
    let saved = f.vcx.read(|cx| {
        serde_json::to_string(&f.workspace.read(cx).serialize_layout(cx)).expect("layout")
    });
    for leaked in [PASSWORD, "YWRtaW4=", "Secret/db", "password"] {
        assert!(
            !saved.contains(leaked),
            "{leaked} reached the layout: {saved}"
        );
    }
    assert!(<ManifestEditor as oxikube_workspace::Item>::serialized_kind().is_none());

    // Closing the tab drops the values; opening the Secret again starts masked.
    let id = editor.entity_id();
    drop(editor);
    f.vcx.update(|window, cx| {
        f.workspace
            .update(cx, |ws, cx| ws.close_item(id, window, cx));
    });
    f.vcx.run_until_parked();
    assert!(f.editors().is_empty());
    f.open(secret_ref(), OpenMode::View);
    let again = f.only_editor();
    assert!(!f.text(&again).contains(PASSWORD));
    assert!(f.text(&again).contains(MASKED));
}

#[gpui::test]
fn a_toggle_never_throws_away_edits(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(secret_ref(), OpenMode::Edit);
    let editor = f.only_editor();
    f.vcx.simulate_input("# keep me\n");
    let edited = f.text(&editor);
    assert!(f.dirty(&editor));

    f.vcx.simulate_keystrokes(alt_v());
    f.vcx.run_until_parked();
    assert_eq!(f.text(&editor), edited, "the buffer is untouched");
    let notice = f
        .vcx
        .read(|cx| editor.read(cx).binding().unwrap().notice().cloned());
    assert!(notice.is_some(), "and the toolbar says why");
    assert!(!f.text(&editor).contains(PASSWORD));
}

#[gpui::test]
fn edit_on_a_read_only_cluster_opens_a_read_only_tab_with_a_notice(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    f.open(deployment_ref(), OpenMode::Edit);
    let editor = f.only_editor();
    assert!(f.read_only(&editor));
    let notice = f
        .vcx
        .read(|cx| editor.read(cx).binding().unwrap().notice().cloned())
        .expect("a notice");
    assert!(notice.contains("read-only"), "{notice}");
}

#[gpui::test]
fn view_yaml_on_a_read_only_cluster_is_plain_reading(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    f.open(secret_ref(), OpenMode::View);
    let editor = f.only_editor();
    let notice = f
        .vcx
        .read(|cx| editor.read(cx).binding().unwrap().notice().cloned());
    assert!(notice.is_none());
    assert!(f.text(&editor).contains(MASKED));
}

#[gpui::test]
fn an_object_that_cannot_be_read_says_so_and_reads_again_on_the_next_open(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let missing = target(gvk("apps", "v1", "Deployment"), "gone");
    f.open(missing.clone(), OpenMode::Edit);
    let editor = f.only_editor();
    assert_eq!(f.title(&editor), "Deployment/gone");
    let load = f
        .vcx
        .read(|cx| editor.read(cx).binding().unwrap().load().clone());
    assert!(matches!(load, ResourceLoad::Failed(_)), "{load:?}");
    assert_eq!(f.text(&editor), "");

    // It exists now: opening it again reads it into the same tab.
    f.resources.insert(
        Resource::from_json(json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "gone", "namespace": "default"}
        }))
        .unwrap(),
    );
    f.open(missing, OpenMode::Edit);
    assert_eq!(f.editors().len(), 1);
    assert!(f.text(&editor).contains("name: gone"));
}

#[gpui::test]
fn the_open_commands_map_to_requests(_: &mut TestAppContext) {
    let t = deployment_ref();
    assert_eq!(
        EditorRequest::of(Command::ResourceEdit { target: t.clone() }),
        Some(EditorRequest::Open {
            target: t.clone(),
            mode: OpenMode::Edit
        })
    );
    assert_eq!(
        EditorRequest::of(Command::ResourceViewYamlInEditor { target: t.clone() }),
        Some(EditorRequest::Open {
            target: t,
            mode: OpenMode::View
        })
    );
}

#[gpui::test]
fn edit_while_the_object_is_still_being_read_keeps_the_tab_locked(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let release = f.hold_next_get();
    f.open(deployment_ref(), OpenMode::View);
    let editor = f.only_editor();
    assert_eq!(f.text(&editor), "", "the get has not returned");

    // "Edit" on the row while the tab reads: nothing can be typed into the empty buffer.
    f.open(deployment_ref(), OpenMode::Edit);
    assert!(f.read_only(&editor), "still locked while reading");
    f.vcx.simulate_input("# lost?\n");
    assert_eq!(
        f.text(&editor),
        "",
        "typing is refused until the object is there"
    );

    release.send(()).expect("the get is waiting");
    f.vcx.run_until_parked();
    assert!(f.text(&editor).contains("replicas: 2"));
    assert!(!f.read_only(&editor), "and the tab is editable once it is");
    assert!(!f.dirty(&editor));
}

#[gpui::test]
fn an_edit_typed_while_a_toggle_renders_is_kept(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    f.open(deployment_ref(), OpenMode::Edit);
    let editor = f.only_editor();
    assert!(!f.text(&editor).contains("managedFields"));

    // The render is asked for, and the user types before it comes back.
    f.vcx
        .update(|window, cx| editor.update(cx, |e, cx| e.toggle_managed_fields(window, cx)));
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| {
            code.api(window, cx).set_text("# typed meanwhile\n")
        })
    });
    f.vcx.run_until_parked();

    assert_eq!(
        f.text(&editor),
        "# typed meanwhile\n",
        "the late render is dropped"
    );
    assert!(f.dirty(&editor), "and the edit still counts");
    let (shown, notice) = f.vcx.read(|cx| {
        let binding = editor.read(cx).binding().unwrap();
        (binding.options().managed_fields, binding.notice().cloned())
    });
    assert!(!shown, "the option goes back to what the buffer shows");
    assert!(notice.is_some(), "and the toolbar says why");
}
