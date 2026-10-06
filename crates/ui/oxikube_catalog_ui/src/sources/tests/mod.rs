//! Tests of the sources screen: rows and inline errors over a scripted backend, the buttons and
//! dialogs in a workspace, and the whole thing over the real service on fakes.

mod buttons;
mod end_to_end;
mod model;
mod paste;
mod rows;
mod settings;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_app::KubeconfigSourcesService;
use oxikube_app::sources::{MemorySourceList, SourceRow};
use oxikube_ports::UserSource;
use oxikube_testkit::{FakeClusterSourcePort, FakeFsPort};
use oxikube_workspace::{OpenOptions, Workspace, test_support::open_workspace};

use super::backend::{ServiceBackend, SourcesBackend};
use super::test_support::ScriptedBackend;
use super::{SourcesDeps, SourcesView};

/// The workspace window with the sources screen open in it.
pub(super) struct Harness {
    pub workspace: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub view: Entity<SourcesView>,
}

impl Harness {
    /// Opens the screen over `backend` and lets its first read finish.
    pub fn open(cx: &mut TestAppContext, backend: Rc<dyn SourcesBackend>) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let deps = SourcesDeps {
            backend,
            workspace: Some(workspace.downgrade()),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| SourcesView::new(deps, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        vcx.run_until_parked();
        Self {
            workspace,
            vcx,
            view: view.expect("the view was built"),
        }
    }

    /// Opens the screen over a scripted backend serving `rows`.
    pub fn scripted(cx: &mut TestAppContext, rows: Vec<SourceRow>) -> (Self, ScriptedBackend) {
        let backend = ScriptedBackend::new(rows);
        (Self::open(cx, Rc::new(backend.clone())), backend)
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&SourcesView) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx)))
    }

    pub fn bounds(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        // GPUI wants a `'static` selector: leak the few a test asks for.
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector)
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        self.bounds(selector).is_some()
    }

    pub fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
        self.bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.vcx.run_until_parked();
    }

    /// Whether the modal layer has a modal open.
    pub fn modal_open(&mut self) -> bool {
        let layer = self
            .workspace
            .read_with(&self.vcx, |ws, _| ws.modal_layer().clone());
        self.vcx.read(|cx| layer.read(cx).has_active_modal())
    }
}

/// The real service over fakes: what the screen runs on in the end-to-end tests.
pub(super) struct Services {
    pub source: Arc<FakeClusterSourcePort>,
    pub fs: Arc<FakeFsPort>,
    pub list: Arc<MemorySourceList>,
    pub service: KubeconfigSourcesService,
}

pub(super) const DIR: &str = "/config/kubeconfigs";

impl Services {
    pub fn new(list: impl IntoIterator<Item = UserSource>) -> Self {
        let source = Arc::new(FakeClusterSourcePort::new());
        let fs = Arc::new(FakeFsPort::new());
        let list = Arc::new(MemorySourceList::new(list));
        let service =
            KubeconfigSourcesService::new(source.clone(), fs.clone(), list.clone(), DIR.into());
        Self {
            source,
            fs,
            list,
            service,
        }
    }

    pub fn backend(&self) -> Rc<dyn SourcesBackend> {
        Rc::new(ServiceBackend::new(self.service.clone()))
    }
}

/// A kubeconfig text the fake validator accepts, with `n` contexts.
pub(super) fn kubeconfig(n: usize) -> String {
    let mut text = String::from("apiVersion: v1\nkind: Config\ncontexts:\n");
    for i in 0..n {
        text.push_str(&format!("- context:\n    cluster: c{i}\n  name: c{i}\n"));
    }
    text
}
