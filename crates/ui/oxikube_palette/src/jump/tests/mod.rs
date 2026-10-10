//! `#[gpui::test]`s of the jump bar, through the shipped keymap, the workspace's modal layer and a
//! host over fakes (a catalog of three contexts, `dev` connected and shown).
//!
//! | File | Covers |
//! |---|---|
//! | `open.rs` | `:` in a table, the bus command, the key context, focus, toggling, closing |
//! | `run.rs` | typing a line and Enter: the commands that go out, errors that stay in the bar |
//! | `complete.rs` | the list of completions, Tab, arrows and Enter on a completion |
//! | `history.rs` | `[`, `]`, `-` in a table and from the bar |
//! | `connect.rs` | a jump to a context that is not connected |
//! | `large.rs` | a cluster with enough kinds that matching runs off the UI thread |
//! | `bus.rs` | the commands on a real `CommandBus` |

mod bus;
mod complete;
mod connect;
mod history;
mod large;
mod open;
mod run;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::executor::block_on;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, TestAppContext,
    VisualTestContext, Window, div,
};
use oxikube_app::search::aliases::AliasRegistry;
use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::kinds::ResourceKind;
use oxikube_keymap::{KeyContextual, KeymapOptions, contexts};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::kinds::core_kinds;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::{
    CommandDispatcher, Item, TabContent, Workspace, test_support::open_workspace,
};
use serde_json::json;

use super::{JumpBar, JumpHost, JumpSources};
use crate::picker::Picker;

/// The cluster of the context `name`.
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

fn namespace(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": { "name": name },
    }))
    .expect("namespace json")
}

/// Records every command the bar sends.
#[derive(Clone, Default)]
pub(super) struct Recorder(pub Rc<RefCell<Vec<Command>>>);

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut App) {
        self.0.borrow_mut().push(command);
    }
}

/// A stand-in for a resource table: a focusable item whose key context is `Table`, where the
/// shipped keymap binds `:`, `[`, `]` and `-`.
struct TableStandIn {
    focus: FocusHandle,
}

impl TableStandIn {
    fn build(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus: cx.focus_handle().tab_stop(true),
        })
    }
}

impl KeyContextual for TableStandIn {
    const KEY_CONTEXT: &'static str = contexts::RESOURCE_TABLE;
}

impl gpui::EventEmitter<oxikube_workspace::ItemEvent> for TableStandIn {}

impl Focusable for TableStandIn {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TableStandIn {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Inside a cluster tab, where the shipped keymap binds `:`.
        div().key_context(contexts::CLUSTER_TAB).size_full().child(
            div()
                .id("table-stand-in")
                .key_context(self.key_context())
                .track_focus(&self.focus)
                .size_full()
                .child("pods"),
        )
    }
}

impl Item for TableStandIn {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(SharedString::from("Pods"))
    }
}

/// A window with the shipped keymap, a table stand-in that has the focus, and a jump host over
/// fakes. `dev` is connected and shown; `prod` and `staging` are not.
pub(super) struct Fixture {
    pub workspace: Entity<Workspace>,
    pub vcx: VisualTestContext,
    pub host: Rc<JumpHost>,
    pub sent: Rc<RefCell<Vec<Command>>>,
    pub sessions: ClusterSessionManager,
    pub connector: Arc<FakeClusterConnectorPort>,
    pub aliases: AliasRegistry,
    pub table_focus: FocusHandle,
}

impl Fixture {
    pub fn new(cx: &mut TestAppContext) -> Self {
        Self::with_kinds(cx, core_kinds())
    }

    /// [`Self::new`] over a `dev` cluster that serves `kinds`.
    pub fn with_kinds(cx: &mut TestAppContext, kinds: Vec<ResourceKind>) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
            super::init(cx);
        });

        let clock = Arc::new(FakeClockPort::default());
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([
            context("dev"),
            context("prod"),
            context("staging"),
        ]));
        let state = Arc::new(FakeStatePort::new());
        let sessions = ClusterSessionManager::new(connector.clone(), source.clone(), clock.clone());
        let catalog = ClusterCatalog::new(source, state.clone(), clock.clone());
        let namespaces = NamespaceService::new(sessions.clone(), state, clock);
        let aliases = AliasRegistry::new();

        // `dev`: connected, serving the stock types and four namespaces.
        let ports = connector.ports_for(&id("dev"));
        ports.discovery.set_kinds(kinds.clone());
        for name in ["default", "kube-system", "monitoring", "web"] {
            ports.resources.insert(namespace(name));
        }
        block_on(sessions.connect(&id("dev"))).expect("connect dev");
        aliases.table(&id("dev")).set_discovered(&kinds);

        let recorder = Recorder::default();
        let sent = recorder.0.clone();
        let sources = JumpSources {
            active: Rc::new(|_| Some(id("dev"))),
            sessions: sessions.clone(),
            catalog,
            aliases: aliases.clone(),
            namespaces,
        };
        let host = Rc::new(JumpHost::new(&workspace, Rc::new(recorder), sources));
        vcx.update(|window, cx| host.install(window, cx));

        let table_focus = vcx.update(|window, cx| {
            let table = TableStandIn::build(cx);
            workspace.update(cx, |ws, cx| ws.open_item(table.clone(), window, cx));
            let focus = table.read(cx).focus_handle(cx);
            focus.focus(window, cx);
            focus
        });
        vcx.run_until_parked();
        Self {
            workspace,
            vcx,
            host,
            sent,
            sessions,
            connector,
            aliases,
            table_focus,
        }
    }

    pub fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
        self.vcx.run_until_parked();
    }

    /// Opens the bar the way the bus does.
    pub fn open(&mut self) {
        let host = self.host.clone();
        self.vcx
            .update(|window, cx| host.apply(super::JumpRequest::Open, window, cx));
        self.settle();
    }

    /// The bar, when it is open.
    pub fn bar(&mut self) -> Option<Entity<JumpBar>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<JumpBar>()
        })
    }

    pub fn picker(&mut self) -> Entity<Picker<super::JumpDelegate>> {
        let bar = self.bar().expect("the jump bar is open");
        self.vcx.update(|_, cx| bar.read(cx).picker().clone())
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&super::JumpDelegate) -> R) -> R {
        let picker = self.picker();
        self.vcx.update(|_, cx| f(&picker.read(cx).delegate))
    }

    pub fn query(&mut self) -> String {
        let picker = self.picker();
        self.vcx.update(|_, cx| picker.read(cx).query(cx))
    }

    pub fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.settle();
    }

    pub fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    /// The commands sent so far, and forgets them.
    pub fn take_sent(&mut self) -> Vec<Command> {
        self.vcx.run_until_parked();
        std::mem::take(&mut *self.sent.borrow_mut())
    }

    /// The messages of the toasts showing in the window.
    pub fn toasts(&mut self) -> Vec<String> {
        let layer = self
            .vcx
            .update(|_, cx| self.workspace.read(cx).toast_layer().clone());
        self.vcx.update(|_, cx| {
            layer
                .read(cx)
                .visible()
                .iter()
                .map(|toast| toast.message.to_string())
                .collect()
        })
    }

    pub fn table_has_focus(&mut self) -> bool {
        let focus = self.table_focus.clone();
        self.vcx.update(|window, _| focus.is_focused(window))
    }

    /// Types `line` in the open bar and presses Enter.
    pub fn run_line(&mut self, line: &str) {
        self.open();
        self.type_text(line);
        self.keys("enter");
    }
}

/// Waits for the contexts and namespaces the bar reads when it opens.
pub(super) fn wait_for_data(f: &mut Fixture) {
    f.settle();
    f.vcx
        .executor()
        .advance_clock(oxikube_runtime::FRAME_INTERVAL * 2);
    f.settle();
}
