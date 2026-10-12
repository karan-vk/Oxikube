//! The cluster, the window and the editors of the apply tests (E10-S08, E10-S09): the real
//! `ApplyService`, `CommandBus` and `MutationGuard` over the testkit's fake cluster ports on the
//! deterministic runtime.

#![allow(dead_code, reason = "each test binary uses a subset")]

use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{App, Entity, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::session::SessionOptions;
use oxikube_app::{ApplyService, ClusterSessionManager, MutationGuard, TemplateService};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_editor::view::{
    ApplyPreviewDialog, ApplyStage, EditorHost, EditorRequest, EditorViewSink, EditorViews,
    EditorViewsDeps, LiveSource, ManifestEditor, SchemaSource,
};
use oxikube_ports::{ClusterContext, ResourceReader, SchemaPort, SourceId};
use oxikube_testkit::fakes::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeResourcePort,
    FakeSchemaPort, FakeStatePort, ResourceCall,
};
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::modal::ModalView;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

pub const CONFIGMAP: &str = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: web
  namespace: shop
data:
  mode: blue
";

pub fn context() -> ClusterContext {
    let name = ContextName::new("kind-oxikube");
    ClusterContext::new(
        ClusterId::new("/kubeconfig", &name),
        name,
        SourceId("kubeconfig".into()),
    )
}

pub fn cluster() -> ClusterId {
    context().cluster
}

/// The editors' commands go to their queue; nothing else is dispatched in these tests.
struct SinkDispatcher(EditorViewSink);

impl CommandDispatcher for SinkDispatcher {
    fn dispatch(&self, command: Command, _: &mut App) {
        if let Some(request) = EditorRequest::of(command) {
            self.0.send(request).expect("the window is open");
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

/// The cluster's fake resource port as the live source of the editors that follow an object.
struct FakeLive(Arc<FakeResourcePort>);

impl LiveSource for FakeLive {
    fn cluster(&self) -> &ClusterId {
        static CLUSTER: std::sync::OnceLock<ClusterId> = std::sync::OnceLock::new();
        CLUSTER.get_or_init(cluster)
    }
    fn reader(&self, _: &App) -> Option<Arc<dyn ResourceReader>> {
        Some(self.0.clone())
    }
}

/// The window's workspace is the cluster's tab; its editors apply through `service`.
struct Host {
    workspace: WeakEntity<Workspace>,
    schemas: Arc<FakeSchemaPort>,
    service: ApplyService,
    sessions: ClusterSessionManager,
    templates: Option<TemplateService>,
    resources: Arc<FakeResourcePort>,
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
    fn apply(&self, _: &ClusterId) -> Option<ApplyService> {
        Some(self.service.clone())
    }
    fn reader(&self, _: &ClusterId) -> Option<Arc<dyn ResourceReader>> {
        Some(self.resources.clone())
    }
    fn cluster_read_only(&self, cluster: &ClusterId) -> bool {
        self.sessions
            .get(cluster)
            .is_some_and(|session| session.read_only())
    }
    fn templates(&self) -> Option<TemplateService> {
        self.templates.clone()
    }
    fn live(&self, _: &ClusterId) -> Option<Rc<dyn LiveSource>> {
        Some(Rc::new(FakeLive(self.resources.clone())))
    }
    fn namespace_selection(&self, cluster: &ClusterId) -> NamespaceSelection {
        self.sessions
            .get(cluster)
            .map(|session| session.namespace_selection().clone())
            .unwrap_or_default()
    }
}

pub struct Fixture {
    pub workspace: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub sessions: ClusterSessionManager,
    pub resources: Arc<FakeResourcePort>,
    pub state: Arc<FakeStatePort>,
    pub sink: EditorViewSink,
    pub _views: Entity<EditorViews>,
}

pub fn setup(cx: &mut TestAppContext, read_only: bool) -> Fixture {
    setup_with(cx, read_only, None)
}

/// [`setup`] whose editors list `templates` in the "Create resource" picker.
pub fn setup_with(
    cx: &mut TestAppContext,
    read_only: bool,
    templates: Option<TemplateService>,
) -> Fixture {
    let (workspace, mut vcx) = open_workspace(cx);
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context()]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
    sessions.open(
        &context(),
        SessionOptions {
            read_only,
            ..SessionOptions::default()
        },
    );
    sessions
        .connect(&cluster())
        .now_or_never()
        .expect("the fakes answer at once")
        .expect("connected");
    let ports = connector.ports_for(&cluster());
    ports
        .discovery
        .set_kinds(oxikube_testkit::kinds::core_kinds());
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::apply", |r| {
            oxikube_app::apply::register_commands(r, sessions.clone())
        })
        .expect("registered");
    let guard = MutationGuard::new(sessions.clone(), state.clone(), clock);
    let bus = CommandBus::new(registry, guard);
    let service = ApplyService::new(bus, sessions.clone(), "alice");

    let (sink, requests) = EditorViewSink::channel();
    let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(SinkDispatcher(sink.clone()));
    let host = Rc::new(Host {
        workspace: workspace.downgrade(),
        schemas: Arc::new(FakeSchemaPort::new()),
        service,
        sessions: sessions.clone(),
        templates,
        resources: ports.resources.clone(),
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
        sessions,
        resources: ports.resources,
        state,
        sink,
        _views: views,
    }
}

pub fn apply_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    }
}

impl Fixture {
    /// A new editor holding `text`, focused.
    pub fn editor_with(&mut self, text: &str) -> Entity<ManifestEditor> {
        self.sink
            .send(EditorRequest::New { cluster: None })
            .unwrap();
        self.vcx.run_until_parked();
        let editor = self
            .vcx
            .read(|cx| self.workspace.read(cx).items_of_type::<ManifestEditor>())
            .pop()
            .expect("an editor opened");
        let code = self.vcx.read(|cx| editor.read(cx).editor().clone());
        self.vcx.update(|window, cx| {
            code.update(cx, |code, cx| code.api(window, cx).set_text(text));
        });
        self.vcx.run_until_parked();
        editor
    }

    pub fn press_apply(&mut self) {
        self.vcx.simulate_keystrokes(apply_key());
        self.vcx.run_until_parked();
    }

    pub fn modal<V: ModalView>(&mut self) -> Option<Entity<V>> {
        self.vcx.read(|cx| {
            self.workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<V>()
        })
    }

    /// `(method, dry_run)` of each write the cluster got.
    pub fn writes(&self) -> Vec<(&'static str, bool)> {
        self.resources
            .mutating_calls()
            .iter()
            .map(|call| {
                let method = match call {
                    ResourceCall::Patch { .. } => "patch",
                    ResourceCall::Replace { .. } => "replace",
                    ResourceCall::Create { .. } => "create",
                    _ => "other",
                };
                (method, call.is_dry_run())
            })
            .collect()
    }

    pub fn notice(&mut self, editor: &Entity<ManifestEditor>) -> String {
        self.vcx.read(|cx| {
            editor
                .read(cx)
                .notice()
                .map(|n| n.text.to_string())
                .unwrap_or_default()
        })
    }

    pub fn stage(&mut self, editor: &Entity<ManifestEditor>) -> ApplyStage {
        self.vcx.read(|cx| editor.read(cx).apply_stage())
    }

    pub fn confirm(&mut self, dialog: &Entity<ApplyPreviewDialog>) {
        dialog.update_in(&mut self.vcx, |dialog, window, cx| {
            dialog.confirm(window, cx)
        });
        self.vcx.run_until_parked();
    }
}
