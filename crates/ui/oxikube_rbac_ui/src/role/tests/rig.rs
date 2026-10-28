//! The window every view test starts from: a workspace, one connectable cluster whose ports are
//! testkit fakes, and a dispatcher that records what the views send.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::executor::block_on;
use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use oxikube_app::store::ResourceStores;
use oxikube_app::{ClusterSessionManager, CoreColumns};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_resources_ui::detail::{DetailDeps, DetailView, Mount};
use oxikube_resources_ui::table::store_runtime;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterPorts, FakeClusterSourcePort,
};
use oxikube_workspace::modal::ModalView;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, OpenOptions, Workspace};
use serde_json::{Value, json};

use crate::role::{DialogMode, RoleDialog, RoleDialogDeps};

/// The cluster of every test.
pub(super) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

/// Records every command the views send.
#[derive(Clone, Default)]
pub(super) struct Recorder(Rc<RefCell<Vec<Command>>>);

impl Recorder {
    pub fn sent(&self) -> Vec<Command> {
        self.0.borrow().clone()
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

pub(super) struct Rig {
    pub vcx: VisualTestContext,
    pub workspace: Entity<Workspace>,
    pub sessions: ClusterSessionManager,
    pub connector: Arc<FakeClusterConnectorPort>,
    pub sent: Recorder,
    clock: Arc<FakeClockPort>,
}

impl Rig {
    pub fn new(cx: &mut TestAppContext) -> Self {
        let entry = ClusterContext::new(
            cluster(),
            ContextName::new("kind"),
            SourceId("kubeconfig".into()),
        );
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_resources_ui::kinds::init(cx);
            crate::init(cx);
        });
        Self {
            vcx,
            workspace,
            sessions,
            connector,
            sent: Recorder::default(),
            clock,
        }
    }

    pub fn ports(&self) -> FakeClusterPorts {
        self.connector.ports_for(&cluster())
    }

    /// Serves `kinds`, stores `objects` and connects.
    pub fn connect(&mut self, kinds: Vec<ResourceKind>, objects: Vec<Resource>) {
        let ports = self.ports();
        ports.discovery.set_kinds(kinds);
        for object in objects {
            ports.resources.insert(object);
        }
        block_on(self.sessions.connect(&cluster())).expect("connect");
        self.vcx.run_until_parked();
    }

    /// Opens the dialog in the workspace's modal layer and lets its reads land.
    pub fn open_dialog(&mut self, mode: DialogMode) -> Entity<RoleDialog> {
        let deps = RoleDialogDeps {
            sessions: self.sessions.clone(),
            dispatcher: Rc::new(self.sent.clone()),
            cluster: cluster(),
        };
        let workspace = self.workspace.clone();
        let dialog = self.vcx.update(|window, cx| {
            let dialog = cx.new(|cx| RoleDialog::new(deps, mode, window, cx));
            workspace.update(cx, |ws, cx| ws.show_modal(dialog.clone(), window, cx));
            dialog
        });
        self.settle();
        dialog
    }

    /// Opens the detail of `target` as a tab.
    pub fn open_detail(&mut self, target: &ResourceRef) -> Entity<DetailView> {
        let deps = self.vcx.update(|_, cx| DetailDeps {
            sessions: self.sessions.clone(),
            stores: Arc::new(ResourceStores::new(store_runtime(self.clock.clone(), cx))),
            columns: Arc::new(CoreColumns::new()),
            dispatcher: Rc::new(self.sent.clone()),
            exec: None,
            scale: None,
            set_image: None,
            rollback: None,
            drain: None,
            forward: None,
        });
        let target = target.clone();
        let workspace = self.workspace.clone();
        let view = self.vcx.update(|window, cx| {
            let view = cx.new(|cx| DetailView::new(target, deps, Mount::Tab, cx));
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(view.clone()), OpenOptions::default(), window, cx);
            });
            view
        });
        self.settle();
        self.settle();
        view
    }

    /// Lets reads, feeds and a redraw land.
    pub fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx.run_until_parked();
    }

    pub fn click(&mut self, selector: &'static str) {
        let at = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center();
        self.vcx.simulate_click(at, Modifiers::none());
        self.settle();
    }

    pub fn is_laid_out(&mut self, selector: &'static str) -> bool {
        self.vcx.debug_bounds(selector).is_some()
    }

    /// The modal of type `T` open in the workspace, if any.
    pub fn modal<T: ModalView>(&mut self) -> Option<Entity<T>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            let layer = workspace.read(cx).modal_layer().clone();
            layer.read(cx).active_modal::<T>()
        })
    }

    /// Runs `run` on `entity`, then lets effects land.
    pub fn with<T: 'static, R>(
        &mut self,
        entity: &Entity<T>,
        run: impl FnOnce(&mut T, &mut gpui::Window, &mut gpui::Context<T>) -> R,
    ) -> R {
        let entity = entity.clone();
        let out = self
            .vcx
            .update(|window, cx| entity.update(cx, |this, cx| run(this, window, cx)));
        self.settle();
        out
    }
}

pub(super) fn role_ref(kind: &str, namespace: Option<&str>, name: &str) -> ResourceRef {
    ResourceRef::new(
        cluster(),
        Gvk::new("rbac.authorization.k8s.io", "v1", kind),
        namespace.map(Into::into),
        name,
    )
}

/// A ClusterRole `name` at `version` with `rules`.
pub(super) fn cluster_role(name: &str, version: &str, rules: Value, extra: Value) -> Resource {
    let mut object = json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": name, "resourceVersion": version, "uid": format!("uid-{name}")},
        "rules": rules,
    });
    if let (Some(extra), Some(target)) = (extra.as_object(), object.as_object_mut()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    Resource::from_json(object).expect("a valid object")
}

/// A Role `demo/name` at `version` with `rules`.
pub(super) fn role(name: &str, version: &str, rules: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
        "metadata": {"name": name, "namespace": "demo", "resourceVersion": version,
                     "uid": format!("uid-{name}")},
        "rules": rules,
    }))
    .expect("a valid object")
}

pub(super) fn pods_rule() -> Value {
    json!([{"apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list"]}])
}
