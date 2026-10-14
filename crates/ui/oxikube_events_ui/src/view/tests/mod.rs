//! The window every events view test starts from: a workspace standing for the cluster's tab, one
//! connected cluster whose ports are testkit fakes, the app's `EventService` on GPUI's background
//! executor fed by `FakeEventsPort`, the shipped keymap, the [`EventsViews`] controller and a
//! dispatcher that does what the bus does with the events commands (records them, then queues
//! them for the controller).

mod filters;
mod keys;
mod live;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::block_on;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, WeakEntity, Window};
use jiff::{SignedDuration, Timestamp};
use oxikube_app::ClusterSessionManager;
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::store::Spawner;
use oxikube_domain::command::Command;
use oxikube_domain::event::Event;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeEventFeed, FakeEventsPort,
    FakeFsPort, FakeStatePort, TICK, event,
};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, Workspace};

use crate::commands::{EventsCommandSink, EventsHost, EventsRequest, EventsViews, EventsViewsDeps};
use crate::model::EventsModel;
use crate::view::{EventsView, EventsViewDeps};

/// The cluster of every test.
pub(crate) fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("kind"))
}

pub(crate) fn t0() -> Timestamp {
    "2026-01-01T00:00:00Z".parse().unwrap()
}

/// Event `uid` about pod `ns/object`, last seen `secs` after t0, `Normal` with `reason`.
pub(crate) fn ev(uid: &str, ns: &str, object: &str, secs: i64, reason: &str) -> Event {
    let mut e = event()
        .uid(uid)
        .reason(reason)
        .message(&format!("message of {uid}"))
        .regarding("Pod", Some(ns), object, &format!("uid-{object}"))
        .build();
    e.namespace = Some(Arc::from(ns));
    e.last_seen = Some(t0() + SignedDuration::from_secs(secs));
    e
}

/// [`ev`] as a `Warning`.
pub(crate) fn warn(uid: &str, ns: &str, object: &str, secs: i64, reason: &str) -> Event {
    let mut e = ev(uid, ns, object, secs, reason);
    e.event_type = oxikube_domain::event::EventType::Warning;
    e
}

/// The four events every test starts with: two warnings, newest first by last seen
/// `w2, n2, w1, n1`.
pub(crate) fn seed() -> Vec<Event> {
    vec![
        ev("n1", "default", "web-0", 1, "Pulled"),
        warn("w1", "default", "web-0", 2, "BackOff"),
        ev("n2", "kube-system", "dns-0", 3, "Started"),
        warn("w2", "shop", "cart-0", 4, "FailedScheduling"),
    ]
}

/// Records every command and does what the bus does with the events commands.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    sent: Rc<RefCell<Vec<Command>>>,
    sink: EventsCommandSink,
}

impl Dispatcher {
    pub(crate) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for Dispatcher {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command.clone());
        if let Some(request) = EventsRequest::of(&command) {
            self.sink.send(request);
        }
    }
}

/// The test's one workspace stands for the cluster's tab.
struct Host(WeakEntity<Workspace>);

impl EventsHost for Host {
    fn workspace(&self, cluster_id: &ClusterId, _: &gpui::App) -> Option<Entity<Workspace>> {
        (cluster_id == &cluster())
            .then(|| self.0.upgrade())
            .flatten()
    }

    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut gpui::App) {}
}

/// One window over fakes. See the [module docs](self).
pub(crate) struct Fx {
    pub(crate) vcx: VisualTestContext,
    pub(crate) workspace: Entity<Workspace>,
    pub(crate) port: Arc<FakeEventsPort>,
    pub(crate) fs: Arc<FakeFsPort>,
    pub(crate) dispatcher: Dispatcher,
    /// The controller; held for the life of the window.
    _views: Entity<EventsViews>,
}

impl Fx {
    /// A window with the cluster connected and its feed holding `seed()`, then `feed`'s ticks.
    pub(crate) fn new(cx: &mut TestAppContext) -> Self {
        Self::with_feed(cx, FakeEventFeed::new().initial(seed()))
    }

    /// [`Self::new`] over `feed`, the state store holding what `state` holds.
    pub(crate) fn with_feed(cx: &mut TestAppContext, feed: FakeEventFeed) -> Self {
        Self::build(cx, feed, Arc::new(FakeStatePort::new()))
    }

    pub(crate) fn build(
        cx: &mut TestAppContext,
        feed: FakeEventFeed,
        state: Arc<FakeStatePort>,
    ) -> Self {
        let entry = ClusterContext::new(cluster(), ContextName::new("kind"), SourceId("k".into()));
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let sessions = ClusterSessionManager::new(
            connector.clone(),
            source,
            Arc::new(FakeClockPort::default()),
        );
        let port = connector.ports_for(&cluster()).events;
        port.install(feed);
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        let clock = Arc::new(FakeClockPort::new(t0() + SignedDuration::from_secs(60)));
        let service = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
        let spawner: Arc<dyn Spawner> = vcx.update(|_, cx| {
            let executor = cx.background_executor().clone();
            Arc::new(move |task| executor.spawn(task).detach()) as Arc<dyn Spawner>
        });
        service.start(spawner);
        block_on(sessions.connect(&cluster())).expect("connect");

        let fs = Arc::new(FakeFsPort::new());
        let (sink, requests) = EventsCommandSink::channel();
        let dispatcher = Dispatcher {
            sent: Rc::default(),
            sink,
        };
        let deps = EventsViewsDeps {
            view: EventsViewDeps {
                service,
                sessions,
                dispatcher: Rc::new(dispatcher.clone()),
                state,
                fs: fs.clone(),
                clock,
            },
            host: Rc::new(Host(workspace.downgrade())),
        };
        let views = vcx.update(|window, cx| EventsViews::start(deps, requests, window, cx));
        vcx.run_until_parked();
        Self {
            vcx,
            workspace,
            port,
            fs,
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

    /// Opens the view (`events::Open`) and lets its first list land.
    pub(crate) fn open(&mut self) -> Entity<EventsView> {
        self.run(Command::EventsOpen {
            cluster: cluster(),
            namespace: None,
            filter: None,
        });
        self.view().expect("the events view is open")
    }

    /// The open view.
    pub(crate) fn view(&mut self) -> Option<Entity<EventsView>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| EventsView::find(&workspace, cx))
    }

    /// Lets tasks run (the feed, the list read off the UI thread) and a frame pass.
    pub(crate) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.executor().advance_clock(Duration::from_millis(20));
        self.vcx.run_until_parked();
    }

    /// Lets the feed's next tick of events arrive.
    pub(crate) fn tick(&mut self) {
        self.port.clock().advance(TICK);
        self.settle();
    }

    /// Reads the view's model.
    pub(crate) fn model<R>(&mut self, read: impl FnOnce(&EventsModel) -> R) -> R {
        let view = self.view().expect("open");
        self.vcx.update(|_, cx| read(view.read(cx).model(cx)))
    }

    /// The uids of the rows, in order.
    pub(crate) fn uids(&mut self) -> Vec<String> {
        self.model(|m| {
            m.events()
                .map(|e| e.uid.as_deref().unwrap_or("-").to_owned())
                .collect()
        })
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
