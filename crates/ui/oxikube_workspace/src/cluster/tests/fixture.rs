//! The shared fixture: a workspace window, a session manager with clusters `prod-eu` and
//! `lab`, a command bus with the real posture commands and a recording `pod::Delete`, and the
//! stand-ins for the tab and the hotbar that S04 builds for real.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Task, TestAppContext, VisualTestContext, Window, div,
};
use oxikube_app::session::SessionOptions;
use oxikube_app::{
    ClusterSession, ClusterSessionManager, CommandBus, CommandOutput, CommandRegistry,
    HandlerContext, MutationGuard, PrefsPatch, PrefsWriter,
};
use oxikube_domain::OxiResult;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ClusterPrefs, ClusterPrefsTable, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use parking_lot::Mutex;

use crate::cluster::{
    BadgeSurface, ClusterBadge, ClusterCommandRunner, ClusterMark, ClusterStatusItem,
    follow_session,
};
use crate::item::{Item, ItemEvent, TabContent};
use crate::status_bar::StatusSide;
use crate::test_support::open_workspace;
use crate::workspace::Workspace;

pub(super) const PROD: &str = "3f2a9c1b7d4e8a60";
pub(super) const LAB: &str = "0011223344556677";

pub(super) fn id(text: &str) -> ClusterId {
    text.parse().unwrap()
}

fn context(cluster: &str, name: &str) -> ClusterContext {
    ClusterContext::new(
        id(cluster),
        ContextName::new(name),
        SourceId("kubeconfig".into()),
    )
}

/// A settings store that keeps what is written and pushes it back into the manager, like the
/// binary's hot reload.
struct StoreWriter {
    manager: ClusterSessionManager,
    values: Mutex<std::collections::HashMap<ClusterId, ClusterPrefs>>,
}

impl PrefsWriter for StoreWriter {
    fn write(
        &self,
        cluster: &ClusterId,
        _: Option<&str>,
        patch: PrefsPatch,
    ) -> BoxFuture<'static, OxiResult<()>> {
        let mut table = ClusterPrefsTable::new(ClusterPrefs::default());
        {
            let mut values = self.values.lock();
            let prefs = values.entry(cluster.clone()).or_default();
            if let Some(read_only) = patch.read_only {
                prefs.read_only = read_only;
            }
            if let Some(colour) = patch.colour {
                prefs.colour = colour;
            }
            for (id, prefs) in values.iter() {
                table = table.with_cluster(id.clone(), prefs.clone());
            }
        }
        self.manager.set_prefs_table(table);
        futures::future::ready(Ok(())).boxed()
    }
}

/// A tab item following one cluster's session: what S04's cluster tab does.
pub(crate) struct ClusterTab {
    focus: FocusHandle,
    pub cluster: ClusterId,
    mark: ClusterMark,
    title: SharedString,
    _follow: Task<()>,
}

impl ClusterTab {
    fn new(manager: &ClusterSessionManager, cluster: ClusterId, cx: &mut Context<Self>) -> Self {
        let session = manager.get(&cluster);
        let follow = follow_session(
            manager,
            cx,
            |this: &Self| Some(this.cluster.clone()),
            |this, session, cx| {
                if let Some(session) = session {
                    this.mark = ClusterMark::of(&session);
                    this.title = session.title().to_owned().into();
                    cx.emit(ItemEvent::UpdateTab);
                }
            },
        );
        Self {
            focus: cx.focus_handle(),
            mark: session.as_ref().map(ClusterMark::of).unwrap_or_default(),
            title: session
                .as_ref()
                .map_or_else(SharedString::default, |s| s.title().to_owned().into()),
            cluster,
            _follow: follow,
        }
    }
}

impl EventEmitter<ItemEvent> for ClusterTab {}

impl Focusable for ClusterTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ClusterTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

impl Item for ClusterTab {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.title.clone()).cluster(self.mark)
    }
}

/// A stand-in for a hotbar entry: the same badge element at the hotbar size.
pub(crate) struct HotbarEntry {
    cluster: ClusterId,
    mark: ClusterMark,
    _follow: Task<()>,
}

impl HotbarEntry {
    fn new(manager: &ClusterSessionManager, cluster: ClusterId, cx: &mut Context<Self>) -> Self {
        let mark = manager
            .get(&cluster)
            .as_ref()
            .map(ClusterMark::of)
            .unwrap_or_default();
        let follow = follow_session(
            manager,
            cx,
            |this: &Self| Some(this.cluster.clone()),
            |this, session: Option<ClusterSession>, cx| {
                this.mark = session.as_ref().map(ClusterMark::of).unwrap_or_default();
                cx.notify();
            },
        );
        Self {
            cluster,
            mark,
            _follow: follow,
        }
    }
}

/// Mounted in the status bar's right group, which is the only surface a test can put a view on
/// before the hotbar exists.
impl crate::status_bar::StatusItem for HotbarEntry {}

impl Render for HotbarEntry {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("hotbar-entry")
            .size(gpui::px(40.))
            .child(ClusterBadge::new(self.mark, BadgeSurface::Hotbar).name("hotbar"))
    }
}

pub(crate) struct Fixture {
    pub ws: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub manager: ClusterSessionManager,
    pub state: Arc<FakeStatePort>,
    pub status: Entity<ClusterStatusItem>,
    pub tab: Entity<ClusterTab>,
    pub runner: ClusterCommandRunner,
    /// How many times the `pod::Debug` handler ran.
    pub debugs: Arc<Mutex<u32>>,
    pub deletes: Arc<Mutex<usize>>,
    /// How many `node::Shell` handlers ran.
    pub node_shells: Arc<Mutex<usize>>,
}

pub(crate) fn bounds(
    vcx: &mut VisualTestContext,
    selector: &'static str,
) -> Option<gpui::Bounds<Pixels>> {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.debug_bounds(selector)
}

/// Opens the window and wires everything for cluster `prod-eu` (`PROD`), connected, with the
/// status item showing it. Sessions follow the settings the test seeds through the manager.
pub(crate) fn fixture(cx: &mut TestAppContext) -> Fixture {
    let (ws, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([context(PROD, "prod-eu"), context(LAB, "lab")]),
    );
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(connector, source, clock.clone());
    manager.open(&context(PROD, "prod-eu"), SessionOptions::default());
    manager.open(&context(LAB, "lab"), SessionOptions::default());
    manager
        .connect(&id(PROD))
        .now_or_never()
        .expect("the fake connects at once")
        .unwrap();

    let deletes = Arc::new(Mutex::new(0));
    let debugs = Arc::new(Mutex::new(0));
    let state = Arc::new(FakeStatePort::new());
    let writer = Arc::new(StoreWriter {
        manager: manager.clone(),
        values: Mutex::default(),
    });
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::posture", |reg| {
            oxikube_app::guard::register_commands(reg, manager.clone(), writer.clone())
        })
        .unwrap();
    let counter = deletes.clone();
    registry
        .register(
            *command::lookup(CommandId::POD_DELETE).unwrap(),
            move |_: Command, cx: HandlerContext| {
                let counter = counter.clone();
                async move {
                    cx.require_mutation()?;
                    *counter.lock() += 1;
                    Ok(CommandOutput::message("Deleted"))
                }
            },
        )
        .unwrap();
    // `pod::Debug` (E09-S10): a mutation the runner confirms with its own wording.
    let debug_counter = debugs.clone();
    registry
        .register(
            *command::lookup(CommandId::POD_DEBUG).unwrap(),
            move |_: Command, cx: HandlerContext| {
                let counter = debug_counter.clone();
                async move {
                    cx.require_mutation()?;
                    *counter.lock() += 1;
                    Ok(CommandOutput::message(
                        "Debug container debugger-x1y2z is running",
                    ))
                }
            },
        )
        .unwrap();
    // `node::Shell` (E09-S09): a mutation the runner confirms with its own wording.
    let node_shells = Arc::new(Mutex::new(0));
    let counter = node_shells.clone();
    registry
        .register(
            *command::lookup(CommandId::NODE_SHELL).unwrap(),
            move |_: Command, cx: HandlerContext| {
                let counter = counter.clone();
                async move {
                    cx.require_mutation()?;
                    *counter.lock() += 1;
                    Ok(CommandOutput::none())
                }
            },
        )
        .unwrap();
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), clock),
    );

    let (status, tab) = vcx.update(|window, cx| {
        let status = cx.new(|cx| ClusterStatusItem::new(manager.clone(), cx));
        status.update(cx, |item, cx| item.set_cluster(Some(id(PROD)), cx));
        ws.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Left, 10, status.clone(), cx);
        });
        let tab = cx.new(|cx| ClusterTab::new(&manager, id(PROD), cx));
        ws.update(cx, |ws, cx| ws.open_item(tab.clone(), window, cx));
        let hotbar = cx.new(|cx| HotbarEntry::new(&manager, id(PROD), cx));
        ws.update(cx, |ws, cx| {
            ws.register_status_item(StatusSide::Right, 0, hotbar.clone(), cx);
        });
        (status, tab)
    });
    let runner = ClusterCommandRunner::new(bus, "me", &ws).with_status_item(&status);
    vcx.run_until_parked();
    Fixture {
        ws,
        vcx,
        manager,
        state,
        status,
        tab,
        runner,
        deletes,
        debugs,
        node_shells,
    }
}
