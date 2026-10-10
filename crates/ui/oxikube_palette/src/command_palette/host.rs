//! [`PaletteHost`]: the palette of one main window, and how `palette::Toggle` reaches it.
//!
//! The binary builds one host per window when it mounts the cluster UI ([`PaletteHost::install`])
//! with the window's workspace, the command bus's index, the dispatcher every view sends commands
//! through, the recents and the session lookup. Three doors lead to it and end in the same
//! [`PaletteHost::apply`]: the `palette::Toggle` GPUI action (the key in the keymap files), the
//! bus command of the same name (menus, agents, the palette's own entry), and a click.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Global, Subscription, Task, Window, WindowId};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_app::{CommandIndex, RecentsStore};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::command_surface::run_on_focused;
use oxikube_workspace::modal::ModalLayerEvent;
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::capture::capture;
use super::delegate::PaletteParts;
use super::outbox::Outbox;
use super::rows::Snapshot;
use super::{CommandPalette, PaletteEnv, Toggle};

/// What a door asks the palette to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteRequest {
    /// Open the palette, or close it when it is open (`palette::Toggle`).
    Toggle,
    /// List the unavailable commands too, or hide them (`palette::ToggleShowAll`).
    ToggleShowAll,
}

impl PaletteRequest {
    /// The request a bus [`Command`] makes; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        match command {
            Command::PaletteToggle => Some(Self::Toggle),
            Command::PaletteToggleShowAll => Some(Self::ToggleShowAll),
            _ => None,
        }
    }

    const ALL: [Self; 2] = [Self::Toggle, Self::ToggleShowAll];

    fn id(self) -> CommandId {
        match self {
            Self::Toggle => CommandId::PALETTE_TOGGLE,
            Self::ToggleShowAll => CommandId::PALETTE_TOGGLE_SHOW_ALL,
        }
    }
}

/// A handle on a window's palette queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct PaletteSink {
    tx: UnboundedSender<PaletteRequest>,
}

impl PaletteSink {
    /// A sink and the receiver the window drains on the UI thread ([`PaletteHost::serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<PaletteRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }
}

/// Registers `palette::Toggle` and `palette::ToggleShowAll` on `registry`, each with its MCP tool
/// stub: `registry.install("oxikube_palette", |r| register_commands(r, sink))`. The handlers only
/// queue the request; the window applies it on the UI thread.
///
/// # Errors
///
/// A [`RegisterError`] when an id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: PaletteSink,
) -> Result<(), RegisterError> {
    for request in PaletteRequest::ALL {
        let id = request.id();
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = PaletteRequest::of(&command)
                    .ok_or_else(|| OxiError::validation("not a palette command"))?;
                send(&sink, request)?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}

fn send(sink: &PaletteSink, request: PaletteRequest) -> OxiResult<()> {
    sink.tx
        .unbounded_send(request)
        .map_err(|_| OxiError::internal("the window with the palette is gone"))
}

/// The palette of one window. See the [`command_palette`](crate::command_palette) module docs.
pub struct PaletteHost {
    workspace: gpui::WeakEntity<Workspace>,
    index: CommandIndex,
    dispatcher: Rc<dyn CommandDispatcher>,
    recents: Arc<dyn RecentsStore>,
    env: Rc<dyn PaletteEnv>,
    /// Sends the confirmed commands when the palette closes; replaced by the next open.
    sender: RefCell<Option<Subscription>>,
}

impl PaletteHost {
    /// The palette of the window whose main workspace is `workspace`, listing `index`'s commands.
    pub fn new(
        workspace: &gpui::Entity<Workspace>,
        index: CommandIndex,
        dispatcher: Rc<dyn CommandDispatcher>,
        recents: Arc<dyn RecentsStore>,
        env: Rc<dyn PaletteEnv>,
    ) -> Self {
        Self {
            workspace: workspace.downgrade(),
            index,
            dispatcher,
            recents,
            env,
            sender: RefCell::new(None),
        }
    }

    /// Makes this the palette of `window`: the `palette::Toggle` key opens it there.
    pub fn install(self: &Rc<Self>, window: &Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        let hosts = cx.default_global::<Hosts>();
        hosts.0.retain(|window, _| open.contains(window));
        hosts.0.insert(id, self.clone());
    }

    /// Does what a door asked, in `window`.
    pub fn apply(&self, request: PaletteRequest, window: &mut Window, cx: &mut App) {
        match request {
            PaletteRequest::Toggle => self.toggle(window, cx),
            PaletteRequest::ToggleShowAll => self.toggle_show_all(window, cx),
        }
    }

    /// Opens the palette over the focused view, or closes it when it is open.
    ///
    /// The focused view and the session are read first (the palette takes the focus), then the
    /// commands are classified once, and the modal opens with its first frame already listing
    /// them.
    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        if open_palette(&workspace, cx).is_some() {
            workspace.update(cx, |workspace, cx| workspace.hide_modal(window, cx));
            return;
        }
        let captured = capture(window, self.env.as_ref(), cx);
        let outbox = Outbox::for_surface(captured.own_commands);
        let parts = PaletteParts {
            snapshot: Snapshot::take(&self.index, &captured.context),
            target: captured.target,
            outbox: outbox.clone(),
            recents: self.recents.clone(),
            workspace: self.workspace.clone(),
        };
        self.send_when_closed(&workspace, outbox, window.window_handle(), cx);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, move |window, cx| {
                CommandPalette::new(parts, window, cx)
            });
        });
    }

    /// Runs what a confirm leaves in `outbox` once the modal layer reports the palette closed.
    ///
    /// The layer hands the focus back in a step it queues just before it announces the close, so
    /// by the time this runs the view the palette opened over has the focus again and the command
    /// acts on it (a terminal command reaches the terminal, a dialog opens over the table).
    /// Escape leaves the outbox empty and runs nothing.
    ///
    /// A command the view runs through its own flow ([`Launch::surface`]) is handed to it one
    /// turn later, with the window (the event carries none); the commands are built and go to the bus only
    /// if the view declines.
    fn send_when_closed(
        &self,
        workspace: &gpui::Entity<Workspace>,
        outbox: Outbox,
        window: gpui::AnyWindowHandle,
        cx: &mut App,
    ) {
        let layer = workspace.read(cx).modal_layer().clone();
        let dispatcher = self.dispatcher.clone();
        let subscription = cx.subscribe(&layer, move |_, event: &ModalLayerEvent, cx| {
            if *event != ModalLayerEvent::Hidden {
                return;
            }
            for launch in outbox.take() {
                let Some(targets) = launch.surface.as_ref().map(|t| t.targets.clone()) else {
                    for command in launch.fallback() {
                        dispatcher.dispatch(command, cx);
                    }
                    continue;
                };
                let dispatcher = dispatcher.clone();
                cx.defer(move |cx| {
                    window
                        .update(cx, |_, window, cx| {
                            if run_on_focused(launch.id, targets, window, cx) {
                                return;
                            }
                            for command in launch.fallback() {
                                dispatcher.dispatch(command, cx);
                            }
                        })
                        .ok();
                });
            }
        });
        *self.sender.borrow_mut() = Some(subscription);
    }

    /// Flips "show all" in the open palette; nothing when it is closed.
    pub fn toggle_show_all(&self, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        if let Some(palette) = open_palette(&workspace, cx) {
            palette.update(cx, |palette, cx| palette.toggle_show_all(window, cx));
        }
    }

    /// Applies each request that arrives on `requests` (the sink [`register_commands`] was given)
    /// in `window` until the window closes. Hold the returned task as long as the window lives.
    pub fn serve(
        self: &Rc<Self>,
        mut requests: UnboundedReceiver<PaletteRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        let host = self.clone();
        window.spawn(cx, async move |cx| {
            while let Some(request) = requests.next().await {
                let applied = cx.update(|window, cx| host.apply(request, window, cx));
                if applied.is_err() {
                    break;
                }
            }
        })
    }
}

/// The palette open in `workspace`'s modal layer, if any.
fn open_palette(
    workspace: &gpui::Entity<Workspace>,
    cx: &App,
) -> Option<gpui::Entity<CommandPalette>> {
    workspace
        .read(cx)
        .modal_layer()
        .read(cx)
        .active_modal::<CommandPalette>()
}

/// The palettes of the open windows.
#[derive(Default)]
struct Hosts(HashMap<WindowId, Rc<PaletteHost>>);

impl Global for Hosts {}

/// The host installed for the active window; with no active window (the OS has not told us yet,
/// a test), the only window that has one.
fn active_host(cx: &App) -> Option<(gpui::AnyWindowHandle, Rc<PaletteHost>)> {
    let hosts = cx.try_global::<Hosts>()?;
    let pick = |window: gpui::AnyWindowHandle| {
        hosts
            .0
            .get(&window.window_id())
            .map(|host| (window, host.clone()))
    };
    if let Some(found) = cx.active_window().and_then(pick) {
        return Some(found);
    }
    let mut windows = cx.windows().into_iter().filter_map(pick);
    let only = windows.next()?;
    windows.next().is_none().then_some(only)
}

/// Binds the `palette::Toggle` action: it opens the palette of the active window. The key itself
/// is in the keymap files. Idempotent.
pub(super) fn register_actions(cx: &mut App) {
    if cx.has_global::<ActionsRegistered>() {
        return;
    }
    cx.set_global(ActionsRegistered);
    cx.on_action(|_: &Toggle, cx| {
        // Action handlers run while the window that dispatched them is borrowed: open once that
        // update has ended, with the focus still where the key found it.
        cx.defer(|cx| {
            let Some((window, host)) = active_host(cx) else {
                return;
            };
            window
                .update(cx, |_, window, cx| host.toggle(window, cx))
                .ok();
        });
    });
}

struct ActionsRegistered;

impl Global for ActionsRegistered {}
