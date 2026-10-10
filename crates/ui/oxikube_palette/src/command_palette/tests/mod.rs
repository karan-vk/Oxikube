//! `#[gpui::test]`s of the command palette, through the shipped keymap, the workspace's modal layer
//! and a recording dispatcher standing in for the bus.
//!
//! | File | Covers |
//! |---|---|
//! | `open.rs` | the key opens and closes it, the `Palette` key context, what is listed with bindings and categories |
//! | `availability.rs` | unavailable commands hidden, "Show all" lists them marked with the reason, confirming one does nothing |
//! | `run.rs` | typing and confirming dispatches the command, recents first, focus returns, a command that needs input |
//! | `order.rs` | the order of matches (unit) |
//! | `capture.rs` | what the palette reads from the focused view and the session |
//! | `speed.rs` | 2 000 commands: virtualised, first frame, keystrokes |

mod availability;
mod capture;
mod open;
mod order;
mod run;
mod speed;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Entity, FocusHandle, Focusable as _, TestAppContext, VisualTestContext,
};
use oxikube_app::{
    ActionContext, CommandIndex, CommandInfo, CommandTarget, MemoryRecents, RecentsStore,
};
use oxikube_domain::Capabilities;
use oxikube_domain::command::{self, Command, CommandId, ViewContext};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_keymap::KeymapOptions;
use oxikube_workspace::command_surface::{self, CommandSurface};
use oxikube_workspace::test_support::{TestItem, open_workspace};
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::{CommandPalette, PaletteEnv, PaletteHost};

/// The key that opens the palette on this OS (the shipped keymap's).
pub(super) const OPEN_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-shift-p"
} else {
    "ctrl-shift-p"
};

/// The key that lists the unavailable commands too, on this OS (the shipped keymap's).
pub(super) const SHOW_ALL_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-shift-a"
} else {
    "ctrl-shift-a"
};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("/kube/config", &ContextName::new("kind-test"))
}

pub(super) fn pod(name: &str) -> ResourceRef {
    ResourceRef::new(
        cluster(),
        Gvk::new("", "v1", "Pod"),
        Some("default".into()),
        name,
    )
}

/// Every declared command, as the app's bus lists them.
pub(super) fn declared_index() -> CommandIndex {
    CommandIndex::new(
        command::COMMANDS
            .iter()
            .map(|meta| CommandInfo::new(meta, "test", true)),
    )
    .expect("declared ids are unique")
}

/// Records the commands the palette sends, and whether the view the palette opened over had the
/// focus again at the moment each was sent.
#[derive(Clone, Default)]
pub(super) struct Recorder {
    commands: Rc<RefCell<Vec<Command>>>,
    watched: Rc<RefCell<Option<FocusHandle>>>,
    focused_at_dispatch: Rc<RefCell<Vec<bool>>>,
}

impl Recorder {
    pub(super) fn commands(&self) -> Vec<Command> {
        self.commands.borrow().clone()
    }

    pub(super) fn ids(&self) -> Vec<CommandId> {
        self.commands.borrow().iter().map(Command::id).collect()
    }

    /// For each dispatch: did the watched view have the focus?
    pub(super) fn focused_at_dispatch(&self) -> Vec<bool> {
        self.focused_at_dispatch.borrow().clone()
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, cx: &mut App) {
        // Like the app's dispatcher: the window is borrowed here, so look one turn later.
        let focus = self.watched.borrow().clone();
        if let (Some(focus), Some(window)) = (focus, cx.active_window()) {
            let log = self.focused_at_dispatch.clone();
            cx.defer(move |cx| {
                window
                    .update(cx, |_, window, _| {
                        log.borrow_mut().push(focus.is_focused(window))
                    })
                    .ok();
            });
        }
        self.commands.borrow_mut().push(command);
    }
}

/// The session the palette reads, scripted.
#[derive(Clone)]
pub(super) struct FakeEnv {
    pub(super) active: Option<ClusterId>,
    pub(super) session: Rc<RefCell<Option<ActionContext>>>,
}

impl FakeEnv {
    fn connected() -> Self {
        Self {
            active: Some(cluster()),
            session: Rc::new(RefCell::new(Some(ActionContext::new(Capabilities::all())))),
        }
    }

    pub(super) fn set_read_only(&self, read_only: bool) {
        let mut session = self.session.borrow_mut();
        *session = session.map(|context| context.read_only(read_only));
    }
}

impl PaletteEnv for FakeEnv {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        self.active.clone()
    }

    fn session(&self, _: &ClusterId, _: &App) -> Option<ActionContext> {
        *self.session.borrow()
    }
}

/// A stand-in for a focused table: says it is a table acting on `target`.
pub(super) struct FakeSurface {
    pub(super) view: ViewContext,
    pub(super) target: CommandTarget,
}

impl CommandSurface for FakeSurface {
    fn view_context(&self) -> ViewContext {
        self.view
    }

    fn command_target(&self, _: &App) -> CommandTarget {
        self.target.clone()
    }
}

/// A workspace window with the shipped keymap, a focused item registered as a table surface, and a
/// palette host over a recording dispatcher.
pub(super) struct Fixture {
    pub(super) workspace: Entity<Workspace>,
    pub(super) vcx: VisualTestContext,
    pub(super) item_focus: FocusHandle,
    pub(super) dispatched: Recorder,
    pub(super) recents: Arc<MemoryRecents>,
    pub(super) env: FakeEnv,
    pub(super) surface: Entity<FakeSurface>,
}

impl Fixture {
    /// A table of pods with `selected` pods selected, on a writable, connected cluster.
    pub(super) fn new(cx: &mut TestAppContext, index: CommandIndex, selected: &[&str]) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        // As the binary does: the shipped keymap is bound after the component library.
        vcx.update(|_, cx| {
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
            crate::command_palette::init(cx);
        });
        let dispatched = Recorder::default();
        let recents = Arc::new(MemoryRecents::new());
        let env = FakeEnv::connected();
        let target = CommandTarget::none()
            .in_cluster(cluster())
            .of_kind(Gvk::new("", "v1", "Pod"))
            .selecting(selected.iter().map(|name| pod(name)).collect());
        let (item_focus, surface) = vcx.update(|window, cx| {
            let item = TestItem::build("Pods", cx);
            workspace.update(cx, |ws, cx| ws.open_item(item.clone(), window, cx));
            let focus = item.read(cx).focus_handle(cx);
            focus.focus(window, cx);
            let surface = cx.new(|_| FakeSurface {
                view: ViewContext::Table,
                target,
            });
            command_surface::register(&surface, &focus, cx);
            *dispatched.watched.borrow_mut() = Some(focus.clone());
            let host = Rc::new(PaletteHost::new(
                &workspace,
                index,
                Rc::new(dispatched.clone()),
                recents.clone() as Arc<dyn RecentsStore>,
                Rc::new(env.clone()),
            ));
            host.install(window, cx);
            (focus, surface)
        });
        vcx.run_until_parked();
        Self {
            workspace,
            vcx,
            item_focus,
            dispatched,
            recents,
            env,
            surface,
        }
    }

    /// The declared commands with `selected` pods selected.
    pub(super) fn declared(cx: &mut TestAppContext, selected: &[&str]) -> Self {
        Self::new(cx, declared_index(), selected)
    }

    pub(super) fn open(&mut self) {
        self.keys(OPEN_KEY);
    }

    pub(super) fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    pub(super) fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.settle();
    }

    pub(super) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    pub(super) fn palette(&mut self) -> Option<Entity<CommandPalette>> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<CommandPalette>()
        })
    }

    pub(super) fn is_open(&mut self) -> bool {
        self.palette().is_some()
    }

    /// The commands listed now, in order.
    pub(super) fn listed(&mut self) -> Vec<CommandId> {
        let palette = self.palette().expect("the palette is open");
        self.vcx
            .update(|_, cx| palette.read(cx).picker().read(cx).delegate.listed())
    }

    pub(super) fn selected(&mut self) -> Option<CommandId> {
        let palette = self.palette().expect("the palette is open");
        self.vcx
            .update(|_, cx| palette.read(cx).picker().read(cx).delegate.selected_id())
    }

    pub(super) fn item_has_focus(&mut self) -> bool {
        let focus = self.item_focus.clone();
        self.vcx.update(|window, _| focus.is_focused(window))
    }

    pub(super) fn toasts(&mut self) -> Vec<String> {
        let workspace = self.workspace.clone();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .into_iter()
                .map(|toast| toast.message.to_string())
                .collect()
        })
    }
}
