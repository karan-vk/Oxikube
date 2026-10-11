//! Shared fixture of the manifest editor's `#[gpui::test]`s (E10-S04, E10-S10): a workspace with
//! `EditorViews`, the shipped keymap, a fake `SchemaPort` on the deterministic runtime, and a bus
//! that records the commands and queues the editor requests.

#![allow(dead_code, reason = "each test binary uses a subset")]

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::schema::JsonSchema;
use oxikube_editor::view::{
    EditorHost, EditorRequest, EditorViewSink, EditorViews, EditorViewsDeps, ManifestEditor,
    SchemaSource,
};
use oxikube_ports::SchemaPort;
use oxikube_testkit::fakes::FakeSchemaPort;
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};
use serde_json::json;

pub fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

/// The bus, reduced to what the editor commands do: record, then queue the request.
pub struct SinkDispatcher {
    sink: EditorViewSink,
    pub sent: Rc<RefCell<Vec<Command>>>,
}

impl CommandDispatcher for SinkDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = EditorRequest::of(command) {
            self.sink.send(request).expect("the window is open");
        }
    }
}

pub struct TestSchemas {
    pub port: Arc<FakeSchemaPort>,
}

impl SchemaSource for TestSchemas {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn label(&self, _: &App) -> gpui::SharedString {
        "kind-oxikube".into()
    }
    fn port(&self, _: &App) -> Option<Arc<dyn SchemaPort>> {
        Some(self.port.clone())
    }
}

/// One window: its own workspace stands for the cluster's tab when `cluster_shown`.
pub struct TestHost {
    pub workspace: WeakEntity<Workspace>,
    pub cluster_shown: bool,
    pub port: Arc<FakeSchemaPort>,
}

impl EditorHost for TestHost {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        self.cluster_shown.then(cluster)
    }
    fn workspace(&self, _: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        self.workspace.upgrade()
    }
    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}
    fn schemas(&self, _: &ClusterId) -> Rc<dyn SchemaSource> {
        Rc::new(TestSchemas {
            port: self.port.clone(),
        })
    }
}

pub struct Fixture {
    pub workspace: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub sent: Rc<RefCell<Vec<Command>>>,
    pub port: Arc<FakeSchemaPort>,
    pub _views: Entity<EditorViews>,
}

pub fn configmap_schema() -> Arc<JsonSchema> {
    Arc::new(JsonSchema::from_value(&json!({
        "type": "object",
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": {"type": "object"},
            "data": {"type": "object", "additionalProperties": {"type": "string"}}
        }
    })))
}

pub fn setup(cx: &mut TestAppContext, cluster_shown: bool) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let port = Arc::new(FakeSchemaPort::new());
    port.insert(
        cluster(),
        Gvk::new("", "v1", "ConfigMap"),
        configmap_schema(),
    );
    let sent = Rc::new(RefCell::new(Vec::new()));
    let (sink, requests) = EditorViewSink::channel();
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(SinkDispatcher {
        sink,
        sent: sent.clone(),
    });
    let host = Rc::new(TestHost {
        workspace: workspace.downgrade(),
        cluster_shown,
        port: port.clone(),
    });
    let views = vcx.update(|window, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_editor::init(cx);
        oxikube_editor::view::install(dispatcher.clone(), cx);
        // The shipped keymap: the tests press the default keys.
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
        sent,
        port,
        _views: views,
    }
}

pub fn new_manifest_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-e"
    } else {
        "ctrl-shift-e"
    }
}

pub fn read_only_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-alt-r"
    } else {
        "ctrl-alt-r"
    }
}

impl Fixture {
    /// Opens a manifest editor with the shipped key and returns it.
    pub fn open_editor(&mut self) -> Entity<ManifestEditor> {
        let editors = |f: &mut Self| {
            f.vcx
                .read(|cx| f.workspace.read(cx).items_of_type::<ManifestEditor>())
        };
        let before = editors(self);
        self.vcx.simulate_keystrokes(new_manifest_key());
        self.vcx.run_until_parked();
        // `items_of_type` has no order: the new editor is the one that was not there.
        editors(self)
            .into_iter()
            .find(|editor| !before.contains(editor))
            .expect("an editor opened")
    }

    pub fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
    }

    pub fn text(&mut self, editor: &Entity<ManifestEditor>) -> String {
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).text().to_string())
        })
    }

    pub fn squiggles(&mut self, editor: &Entity<ManifestEditor>) -> Vec<Range<usize>> {
        self.vcx.read(|cx| {
            let code = editor.read(cx).editor().read(cx);
            code.state()
                .read(cx)
                .diagnostics()
                .map(|set| set.iter().map(|entry| entry.range.clone()).collect())
                .unwrap_or_default()
        })
    }

    pub fn advance(&mut self, by: Duration) {
        self.vcx.executor().advance_clock(by);
        self.vcx.run_until_parked();
    }
}
