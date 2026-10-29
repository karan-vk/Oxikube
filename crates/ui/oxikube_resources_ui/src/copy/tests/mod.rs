//! The copy flow in a window: the cluster picker, the namespace picker and the review dialog over
//! the real guard, the real `resource::CopyTo` handler and two fake clusters.

mod flow;
mod review;

use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::{ClusterSessionManager, CopyService, MutationGuard};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeResourcePort, FakeStatePort,
};
use oxikube_workspace::modal::ModalView;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{ToastLevel, Workspace};
use serde_json::{Value, json};

use super::{ClusterPicker, CopyDialog, CopyFlow, NamespacePicker};

pub(super) fn id(name: &str) -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new(name))
}

fn context(name: &str) -> ClusterContext {
    ClusterContext::new(
        id(name),
        ContextName::new(name),
        SourceId("kubeconfig".into()),
    )
}

/// A window over clusters `a` (the source) and `b` (the target).
pub(super) struct Rig {
    pub vcx: VisualTestContext,
    pub workspace: Entity<Workspace>,
    pub sessions: ClusterSessionManager,
    pub connector: Arc<FakeClusterConnectorPort>,
    pub state: Arc<FakeStatePort>,
    pub flow: CopyFlow,
}

impl Rig {
    pub fn new(cx: &mut TestAppContext) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        let source =
            Arc::new(FakeClusterSourcePort::new().with_contexts([context("a"), context("b")]));
        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let state = Arc::new(FakeStatePort::new());
        let mut registry = CommandRegistry::new();
        registry
            .install("oxikube_app::copy", |r| {
                oxikube_app::copy::register_commands(r, sessions.clone())
            })
            .expect("copy registers");
        registry
            .install(
                "oxikube_app::namespaces",
                oxikube_app::namespaces::register_commands,
            )
            .expect("namespaces register");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(sessions.clone(), state.clone(), clock),
        );
        let service = CopyService::new(bus, sessions.clone(), "alice");
        let flow = CopyFlow::new(service, workspace.downgrade());
        let rig = Self {
            vcx,
            workspace,
            sessions,
            connector,
            state,
            flow,
        };
        for name in ["a", "b"] {
            rig.connector
                .ports_for(&id(name))
                .discovery
                .set_kinds(oxikube_testkit::kinds::core_kinds());
        }
        rig.cluster("a")
            .insert(object(json!({"apiVersion": "v1", "kind": "Namespace",
            "metadata": {"name": "shop"}})));
        for namespace in ["prod", "staging"] {
            rig.cluster("b")
                .insert(object(json!({"apiVersion": "v1", "kind": "Namespace",
                "metadata": {"name": namespace}})));
        }
        rig.connect("a");
        rig.connect("b");
        rig
    }

    pub fn connect(&self, name: &str) {
        block_on(self.sessions.connect(&id(name))).expect("connect");
    }

    pub fn cluster(&self, name: &str) -> Arc<FakeResourcePort> {
        self.connector.ports_for(&id(name)).resources
    }

    /// Puts `value` into cluster `a` and returns its reference.
    pub fn source(&self, value: Value) -> ResourceRef {
        let text = |pointer: &str| {
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let gvk = Gvk::from_api_version(
            &text("/apiVersion").unwrap_or_default(),
            &text("/kind").unwrap_or_default(),
        );
        let name = text("/metadata/name").unwrap_or_default();
        let reference = match text("/metadata/namespace") {
            Some(namespace) => ResourceRef::namespaced(id("a"), gvk, namespace, name),
            None => ResourceRef::cluster_scoped(id("a"), gvk, name),
        };
        self.cluster("a").insert(object(value));
        reference
    }

    pub fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(oxikube_runtime::FRAME_INTERVAL * 2);
        self.vcx.run_until_parked();
    }

    pub fn begin(&mut self, source: ResourceRef) {
        let flow = self.flow.clone();
        self.vcx.update(|window, cx| {
            assert!(flow.begin(source, window, cx), "a picker opened");
        });
        self.settle();
    }

    fn modal<V: ModalView>(&mut self) -> Option<Entity<V>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            let layer = workspace.read(cx).modal_layer().clone();
            layer.read(cx).active_modal::<V>()
        })
    }

    pub fn cluster_picker(&mut self) -> Option<Entity<ClusterPicker>> {
        self.modal()
    }

    pub fn namespace_picker(&mut self) -> Option<Entity<NamespacePicker>> {
        self.modal()
    }

    pub fn dialog(&mut self) -> Option<Entity<CopyDialog>> {
        self.modal()
    }

    /// The visible toasts, as `(level, message)`.
    pub fn toasts(&mut self) -> Vec<(ToastLevel, String)> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            let layer = workspace.read(cx).toast_layer().clone();
            layer
                .read(cx)
                .visible()
                .into_iter()
                .map(|toast| (toast.level, toast.message.to_string()))
                .collect()
        })
    }

    /// Writes that reached cluster `name` for real.
    pub fn real_writes(&self, name: &str) -> usize {
        self.cluster(name)
            .mutating_calls()
            .iter()
            .filter(|call| !call.is_dry_run())
            .count()
    }
}

pub(super) fn object(value: Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

pub(super) fn configmap() -> Value {
    json!({"apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "shop", "uid": "u", "resourceVersion": "4"},
        "data": {"mode": "blue"}})
}
