//! The window every subject page test starts from: a workspace standing for the cluster's tab,
//! the recorded RBAC fixtures as the policy, the shipped keymap, the [`SubjectsViews`] controller
//! and a dispatcher that does what the bus does with the `rbac::*` commands (records them, then
//! queues them for the controller).

mod keys;
mod matrix;
mod pages;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, WeakEntity, Window};
use oxikube_app::rbac_subjects::PolicyHandle;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_keymap::KeymapOptions;
use oxikube_testkit::fixtures::rbac;
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::{
    RbacRequest, StaticBackend, SubjectsBackend, SubjectsHost, SubjectsRequestSink, SubjectsView,
    SubjectsViews, SubjectsViewsDeps,
};

/// The namespace the fixtures' RoleBindings live in.
pub(crate) const NS: &str = "oxi-rbac-fx";

/// The cluster of every test.
pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

/// Records every command and does what the bus does with the `rbac::*` commands.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
    sink: SubjectsRequestSink,
}

impl Dispatcher {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for Dispatcher {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = RbacRequest::of(&command) {
            self.sink.send(request);
        }
    }
}

/// The test's one workspace stands for the cluster's tab.
struct Host(WeakEntity<Workspace>);

impl SubjectsHost for Host {
    fn workspace(&self, id: &ClusterId, _: &gpui::App) -> Option<Entity<Workspace>> {
        (id == &cluster()).then(|| self.0.upgrade()).flatten()
    }

    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut gpui::App) {}
}

/// One window over the fixtures. See the [module docs](self).
pub(crate) struct Fx {
    pub(crate) vcx: VisualTestContext,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) dispatcher: Dispatcher,
    _views: Entity<SubjectsViews>,
}

impl Fx {
    pub(crate) fn new(cx: &mut TestAppContext) -> Self {
        Self::over(cx, PolicyHandle::fixed(&rbac::cluster()))
    }

    pub(crate) fn over(cx: &mut TestAppContext, policy: PolicyHandle) -> Self {
        Self::on(cx, Rc::new(StaticBackend::new(cluster(), policy)))
    }

    /// A window whose pages read from `backend`.
    pub(crate) fn on(cx: &mut TestAppContext, backend: Rc<dyn SubjectsBackend>) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        let (sink, requests) = SubjectsRequestSink::channel();
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
        };
        let deps = SubjectsViewsDeps {
            backends: Rc::new(move |_| backend.clone()),
            dispatcher: Rc::new(dispatcher.clone()),
            host: Rc::new(Host(workspace.downgrade())),
        };
        let views = vcx.update(|window, cx| SubjectsViews::start(deps, requests, window, cx));
        vcx.run_until_parked();
        Self {
            vcx,
            workspace,
            dispatcher,
            _views: views,
        }
    }

    /// Sends `command` as the bus would, and lets it land.
    pub(crate) fn run(&mut self, command: Command) {
        let dispatcher = self.dispatcher.clone();
        self.vcx.update(|_, cx| dispatcher.dispatch(command, cx));
        self.settle();
    }

    /// Opens the Users page.
    pub(crate) fn users(&mut self) -> Entity<SubjectsView> {
        self.run(Command::RbacOpenUsers { cluster: cluster() });
        self.view().expect("the subjects page is open")
    }

    /// Opens the Groups page.
    pub(crate) fn groups(&mut self) -> Entity<SubjectsView> {
        self.run(Command::RbacOpenGroups { cluster: cluster() });
        self.view().expect("the subjects page is open")
    }

    /// Opens the matrix of `subject`.
    pub(crate) fn subject(&mut self, subject: &str) -> Entity<SubjectsView> {
        self.run(Command::RbacShowSubject {
            cluster: cluster(),
            subject: subject.to_owned(),
        });
        self.view().expect("the subjects page is open")
    }

    /// The open page.
    pub(crate) fn view(&mut self) -> Option<Entity<SubjectsView>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| SubjectsView::find(&workspace, cx))
    }

    /// Lets tasks run (the background reads) and a frame pass.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(Duration::from_millis(200));
        self.vcx.run_until_parked();
    }

    /// Types `keys` into the focused element.
    pub(crate) fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    pub(crate) fn drawn(&mut self, selector: &'static str) -> bool {
        self.vcx.debug_bounds(selector).is_some()
    }

    /// Clicks the element with `selector`.
    pub(crate) fn click(&mut self, selector: &'static str) {
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        self.vcx.simulate_click(bounds.center(), Modifiers::none());
        self.settle();
    }
}
