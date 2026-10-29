//! Tests of the apply panel (E20-S05): the model on its own, and the panel in a cluster tab over
//! the real `ApplyService`, `CommandBus` and `MutationGuard` on the testkit's fake cluster ports,
//! file system and kustomize.

mod flow;
mod kustomize;
mod model;
mod open;
mod read_only;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, Modifiers, Task};
use oxikube_app::apply::{ApplyService, register_commands as register_apply};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::kustomize::KustomizeService;
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::kustomize::{KustomizeTool, KustomizeToolKind};
use oxikube_ports::ClockPort;
use oxikube_testkit::{FakeKustomizePort, FakeStatePort};
use oxikube_workspace::Workspace;

use super::{
    ApplyBackend, ApplyRequest, ApplyRequestSink, ApplyView, ApplyViews, ApplyViewsDeps,
    EditorDocument, ServiceBackend, SourceKind, register_commands,
};
use crate::table::tests::fixture::{Fixture, cluster};

pub(super) const CM_A: &str =
    "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\ndata:\n  k: v\n";
pub(super) const CM_B: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: b\n";
pub(super) const NS: &str = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: shop\n";

pub(super) fn tool() -> KustomizeTool {
    KustomizeTool {
        kind: KustomizeToolKind::Standalone,
        version: "v5.4.2".into(),
        path: "/usr/local/bin/kustomize".into(),
    }
}

/// The window, the cluster tab, the panel's views and the fakes behind them.
pub(super) struct Rig {
    pub f: Fixture,
    pub sink: ApplyRequestSink,
    pub kustomize: Arc<FakeKustomizePort>,
    pub opened: Rc<RefCell<Vec<EditorDocument>>>,
    pub bus: CommandBus,
    pub state: Arc<FakeStatePort>,
    _views: Task<()>,
}

impl Rig {
    /// Cluster `kind` connected (core kinds served), `kustomize` detected when `tool` is given.
    pub fn new(cx: &mut gpui::TestAppContext, tool: Option<KustomizeTool>) -> Self {
        Self::wrapped(cx, tool, |backend| Rc::new(backend))
    }

    /// [`Self::new`] over a backend `wrap`ped around the real one.
    pub fn wrapped(
        cx: &mut gpui::TestAppContext,
        tool: Option<KustomizeTool>,
        wrap: impl FnOnce(ServiceBackend) -> Rc<dyn ApplyBackend>,
    ) -> Self {
        let mut f = Fixture::new(cx);
        f.ports()
            .discovery
            .set_kinds(oxikube_testkit::kinds::core_kinds());
        block_on(f.sessions.connect(&cluster())).expect("connect");
        f.vcx.run_until_parked();

        let sessions: ClusterSessionManager = f.sessions.clone();
        let mut registry = CommandRegistry::new();
        registry
            .install("oxikube_app::apply", |r| {
                register_apply(r, sessions.clone())
            })
            .expect("registered");
        let (sink, requests) = ApplyRequestSink::channel();
        registry
            .install("oxikube_resources_ui::apply", |r| {
                register_commands(r, sink.clone())
            })
            .expect("registered");
        let clock: Arc<dyn ClockPort> = f.clock.clone();
        let guard = MutationGuard::new(sessions.clone(), f.state.clone(), clock);
        let bus = CommandBus::new(registry, guard);
        let service = ApplyService::new(bus.clone(), sessions.clone(), "alice");

        let kustomize = Arc::new(match tool {
            Some(tool) => FakeKustomizePort::with_tool(tool),
            None => FakeKustomizePort::new(),
        });
        let backend = ServiceBackend::new(
            service,
            f.fs.clone(),
            Some(KustomizeService::new(kustomize.clone())),
            f.state.clone(),
            sessions,
        );
        let opened: Rc<RefCell<Vec<EditorDocument>>> = Rc::default();
        let record = opened.clone();
        let deps = ApplyViewsDeps {
            backend: wrap(backend),
            editor: Rc::new(move |document, _| record.borrow_mut().push(document)),
            tabs: f.tabs.downgrade(),
        };
        let views = f
            .vcx
            .update(|window, cx| ApplyViews::start(deps, requests, window, cx));
        let state = f.state.clone();
        let mut rig = Self {
            f,
            sink,
            kustomize,
            opened,
            bus,
            state,
            _views: views,
        };
        rig.settle();
        rig
    }

    /// Puts a file in the fake file system.
    pub fn file(&self, path: &str, text: &str) {
        self.f.fs.insert(path, text);
    }

    /// Lets the loads, the streams and a coalesced redraw land.
    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.f.settle();
        }
    }

    /// Opens the panel on `path` as `apply::Directory` would.
    pub fn open(&mut self, path: &str) {
        self.request(SourceKind::Manifests, Some(path));
    }

    pub fn request(&mut self, kind: SourceKind, path: Option<&str>) {
        self.sink
            .send(ApplyRequest {
                cluster: cluster(),
                kind,
                path: path.map(str::to_owned),
                namespace: None,
            })
            .expect("the window is open");
        self.settle();
    }

    /// The tab's workspace.
    pub fn workspace(&mut self) -> Entity<Workspace> {
        let tabs = self.f.tabs.clone();
        self.f
            .vcx
            .read(|cx| {
                tabs.read(cx)
                    .tab(&cluster())
                    .map(|t| t.read(cx).workspace().clone())
            })
            .expect("the cluster has a tab")
    }

    /// The open panel.
    pub fn view(&mut self) -> Entity<ApplyView> {
        let workspace = self.workspace();
        self.f
            .vcx
            .read(|cx| workspace.read(cx).items_of_type::<ApplyView>())
            .pop()
            .expect("the panel is open")
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&ApplyView) -> R) -> R {
        let view = self.view();
        self.f.vcx.read(|cx| f(view.read(cx)))
    }

    pub fn update(&mut self, f: impl FnOnce(&mut ApplyView, &mut gpui::Context<ApplyView>)) {
        let view = self.view();
        self.f.vcx.update(|_, cx| view.update(cx, f));
        self.settle();
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.f.vcx.debug_bounds(selector).is_some()
    }

    pub fn click(&mut self, selector: &str) {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let at = self
            .f
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center();
        self.f.vcx.simulate_click(at, Modifiers::none());
        self.settle();
    }

    /// The status words of the rows, in plan order.
    pub fn words(&mut self) -> Vec<&'static str> {
        self.read(|v| {
            v.model()
                .rows()
                .iter()
                .map(|row| row.shown().map_or("waiting", |s| s.word()))
                .collect()
        })
    }

    /// `(method, dry_run)` of each write the cluster got.
    pub fn writes(&self) -> Vec<bool> {
        self.f
            .ports()
            .resources
            .mutating_calls()
            .iter()
            .map(|call| call.is_dry_run())
            .collect()
    }
}

/// A directory of manifests in the fake file system.
pub(super) fn seed(rig: &Rig, dir: &str, files: &[(&str, &str)]) {
    for (name, text) in files {
        rig.file(&Path::new(dir).join(name).display().to_string(), text);
    }
}
