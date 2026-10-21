//! Tests of the charts page: the model's wording and the page in a workspace window over the
//! real `ChartsService` on the testkit's `FakeHelmPort`, with a dispatcher that records commands.

mod model;
mod page;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_app::{ChartsService, HelmReposService};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{ChartHit, ChartRef, HelmCapabilities, HelmVersion, RepoEntry};
use oxikube_testkit::{FakeClockPort, FakeHelmPort};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, OpenOptions};

use super::{ChartsDeps, HelmChartsView, ServiceBackend};

pub(super) fn hit(repo: &str, name: &str, version: &str, app: &str, desc: &str) -> ChartHit {
    ChartHit {
        chart: ChartRef::new(name, version).in_repo(repo),
        app_version: (!app.is_empty()).then(|| app.to_owned()),
        description: desc.to_owned(),
    }
}

pub(super) fn repo(name: &str) -> RepoEntry {
    RepoEntry {
        name: name.into(),
        url: format!("https://{name}.example.com"),
    }
}

/// Records every command; nothing runs.
#[derive(Clone, Default)]
pub(super) struct TestBus {
    sent: Rc<RefCell<Vec<Command>>>,
}

impl TestBus {
    pub fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for TestBus {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command);
    }
}

pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<HelmChartsView>,
    pub fake: Arc<FakeHelmPort>,
    pub repos: HelmReposService,
    pub bus: TestBus,
}

impl Harness {
    /// Opens the page over a `helm` that lists `repos` and finds `hits` for the first search.
    pub fn open(cx: &mut TestAppContext, repos: &[&str], hits: Vec<ChartHit>) -> Self {
        let fake = Arc::new(FakeHelmPort::with_capabilities(HelmCapabilities::full(
            HelmVersion::new(3, 14, 0),
        )));
        fake.script()
            .repos
            .push_ok(repos.iter().map(|n| repo(n)).collect());
        fake.script().search.push_ok(hits);
        Self::open_with(cx, fake)
    }

    pub fn open_with(cx: &mut TestAppContext, fake: Arc<FakeHelmPort>) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let repos = HelmReposService::new(
            fake.clone(),
            fake.clone(),
            Arc::new(FakeClockPort::default()),
        );
        let charts = ChartsService::new(fake.clone(), repos.catalog_changes());
        let bus = TestBus::default();
        let deps = ChartsDeps {
            backend: Rc::new(ServiceBackend::new(charts)),
            dispatcher: Rc::new(bus.clone()),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| HelmChartsView::new(deps, window, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        let mut harness = Self {
            vcx,
            view: view.expect("built"),
            fake,
            repos,
            bus,
        };
        harness.settle();
        harness
    }

    /// A page with no `helm` binary.
    pub fn open_without_helm(cx: &mut TestAppContext) -> Self {
        Self::open_with(
            cx,
            Arc::new(FakeHelmPort::with_capabilities(
                HelmCapabilities::native_only(),
            )),
        )
    }

    /// Runs what is ready and lets the redraws land.
    pub fn settle(&mut self) {
        for _ in 0..4 {
            self.vcx.run_until_parked();
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&HelmChartsView) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx)))
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector).is_some()
    }

    pub fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.settle();
    }

    /// Types `text` into the search field, key by key.
    pub fn type_query(&mut self, text: &str) {
        let input = self.read(|v| v.query.clone());
        self.vcx
            .update(|window, cx| input.update(cx, |i, cx| i.focus(window, cx)));
        self.vcx.simulate_input(text);
        self.settle();
    }
}
