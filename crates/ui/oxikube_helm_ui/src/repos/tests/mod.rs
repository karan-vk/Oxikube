//! Tests of the repositories page: the model's wording, the form, and the page in a workspace
//! window over the real `HelmReposService` on the testkit's `FakeHelmPort`, with a dispatcher
//! that plays the bus for the three `helm::Repo*` commands.

mod form;
mod model;
mod page;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, Modifiers, Point, Task, TestAppContext, VisualTestContext};
use oxikube_app::HelmReposService;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmCapabilities, RepoEntry};
use oxikube_testkit::{FakeClockPort, FakeHelmPort};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, OpenOptions};

use super::{HelmReposView, ReposDeps, ServiceBackend};

pub(super) fn entry(name: &str) -> RepoEntry {
    RepoEntry {
        name: name.into(),
        url: format!("https://{name}.example.com"),
    }
}

/// Records every command and runs the repository ones on the service, as the bus would.
#[derive(Clone)]
pub(super) struct TestBus {
    sent: Rc<RefCell<Vec<Command>>>,
    service: HelmReposService,
}

impl TestBus {
    pub fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for TestBus {
    fn dispatch(&self, command: Command, cx: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        let service = self.service.clone();
        cx.spawn(async move |_| {
            // Failures are in the service's snapshot, which the page shows.
            match command {
                Command::HelmRepoAdd {
                    name,
                    url,
                    username,
                    insecure_skip_tls_verify,
                    ca_file,
                } => {
                    let mut request = oxikube_domain::helm::RepoAddRequest::new(name, url);
                    request.username = username;
                    request.insecure_skip_tls_verify = insecure_skip_tls_verify;
                    request.ca_file = ca_file;
                    service.add(request).await.ok();
                }
                Command::HelmRepoRemove { name } => {
                    service.remove(&name).await.ok();
                }
                Command::HelmRepoUpdate { names } => {
                    service.update(&names).await.ok();
                }
                _ => {}
            }
        })
        .detach();
    }
}

pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<HelmReposView>,
    pub fake: Arc<FakeHelmPort>,
    pub service: HelmReposService,
    pub bus: TestBus,
    _keep: Option<Task<()>>,
}

impl Harness {
    /// Opens the page; `repos` is what Helm lists.
    pub fn open(cx: &mut TestAppContext, repos: &[&str]) -> Self {
        let fake = Arc::new(FakeHelmPort::new());
        fake.script()
            .repos
            .push_ok(repos.iter().map(|n| entry(n)).collect());
        Self::open_with(cx, fake)
    }

    pub fn open_with(cx: &mut TestAppContext, fake: Arc<FakeHelmPort>) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let service = HelmReposService::new(
            fake.clone(),
            fake.clone(),
            Arc::new(FakeClockPort::default()),
        );
        let bus = TestBus {
            sent: Rc::default(),
            service: service.clone(),
        };
        let deps = ReposDeps {
            backend: Rc::new(ServiceBackend::new(service.clone())),
            dispatcher: Rc::new(bus.clone()),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| HelmReposView::new(deps, window, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        let mut harness = Self {
            vcx,
            view: view.expect("built"),
            fake,
            service,
            bus,
            _keep: None,
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
        for _ in 0..3 {
            self.vcx.run_until_parked();
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&HelmReposView) -> R) -> R {
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

    /// The add form as the inputs hold it.
    pub fn form(&mut self) -> crate::repos::AddForm {
        let view = self.view.clone();
        self.vcx.update(|_, cx| view.read(cx).form(cx))
    }

    /// Types `text` into one of the form's inputs.
    pub fn type_into(
        &mut self,
        pick: impl FnOnce(&HelmReposView) -> Entity<oxikube_ui::input::InputState>,
        text: &str,
    ) {
        let input = self.read(pick);
        let text = text.to_owned();
        self.vcx
            .update(|window, cx| input.update(cx, |i, cx| i.set_value(text, window, cx)));
        self.settle();
    }
}
