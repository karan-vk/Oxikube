//! [`HelpHost`]: the help overlay of one window, and how `help::Show` reaches it.
//!
//! The binary builds one host per window when it mounts the cluster UI ([`HelpHost::install`]).
//! Three doors lead to it and end in [`HelpHost::toggle`]: the `help::Show` GPUI action (the `?`
//! key in the keymap files, and `?` again inside the overlay), the bus command of the same name
//! (the command palette, agents), and nothing else; the overlay lists keys and runs no command.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Entity, Global, Task, WeakEntity, Window, WindowId};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::Workspace;

use super::Show;
use super::model::HelpModel;
use super::overlay::HelpOverlay;

/// A handle on a window's help queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct HelpSink {
    tx: UnboundedSender<()>,
}

impl HelpSink {
    /// A sink and the receiver the window drains on the UI thread ([`HelpHost::serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<()>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }
}

/// Registers `help::Show` on `registry` with its MCP tool stub:
/// `registry.install("oxikube_palette", |r| register_commands(r, sink))`. The handler only queues
/// the request; the window shows the overlay on the UI thread. Read-only: no guard tier applies.
///
/// # Errors
///
/// A [`RegisterError`] when the id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: HelpSink,
) -> Result<(), RegisterError> {
    let id = CommandId::HELP_SHOW;
    let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            if !matches!(command, Command::HelpShow) {
                return Err(OxiError::validation("not the help::Show command"));
            }
            send(&sink)?;
            Ok(CommandOutput::none())
        }
    })
}

fn send(sink: &HelpSink) -> OxiResult<()> {
    sink.tx
        .unbounded_send(())
        .map_err(|_| OxiError::internal("the window with the help overlay is gone"))
}

/// The help overlay of one window. See the module docs.
pub struct HelpHost {
    workspace: WeakEntity<Workspace>,
}

impl HelpHost {
    /// The help of the window whose main workspace is `workspace`.
    pub fn new(workspace: &Entity<Workspace>) -> Self {
        Self {
            workspace: workspace.downgrade(),
        }
    }

    /// Makes this the help of `window`: the `help::Show` key opens it there.
    pub fn install(self: &Rc<Self>, window: &Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        let hosts = cx.default_global::<Hosts>();
        hosts.0.retain(|window, _| open.contains(window));
        hosts.0.insert(id, self.clone());
    }

    /// Opens the overlay over the focused view, or closes it when it is open. Another modal that
    /// is open (a pending confirmation, a running delete) is left alone: the overlay only lists
    /// keys, so it never takes the place of a decision, and the modal layer would replace the
    /// modal without asking it.
    ///
    /// The bindings are resolved first, from the focus path as it is now (the overlay takes the
    /// focus), once: the overlay's first frame already lists them.
    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let (open, other_modal) = {
            let layer = workspace.read(cx).modal_layer().read(cx);
            let open = layer.active_modal::<HelpOverlay>().is_some();
            (open, !open && layer.has_active_modal())
        };
        if open {
            workspace.update(cx, |workspace, cx| workspace.hide_modal(window, cx));
            return;
        }
        if other_modal {
            return;
        }
        let model = Arc::new(HelpModel::capture(window, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, move |window, cx| {
                HelpOverlay::new(model, window, cx)
            });
        });
    }

    /// Toggles the overlay for each request that arrives on `requests` (the sink
    /// [`register_commands`] was given) in `window`, until the window closes. Hold the returned
    /// task as long as the window lives.
    pub fn serve(
        self: &Rc<Self>,
        mut requests: UnboundedReceiver<()>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        let host = self.clone();
        window.spawn(cx, async move |cx| {
            while requests.next().await.is_some() {
                if cx.update(|window, cx| host.toggle(window, cx)).is_err() {
                    break;
                }
            }
        })
    }
}

/// The help overlays of the open windows.
#[derive(Default)]
struct Hosts(HashMap<WindowId, Rc<HelpHost>>);

impl Global for Hosts {}

/// The host installed for the active window; with no active window (the OS has not told us yet, a
/// test), the only window that has one.
fn active_host(cx: &App) -> Option<(gpui::AnyWindowHandle, Rc<HelpHost>)> {
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

/// Binds the `help::Show` action: it toggles the overlay of the active window. The keys are in the
/// keymap files. Idempotent.
pub(super) fn register_actions(cx: &mut App) {
    if cx.has_global::<ActionsRegistered>() {
        return;
    }
    cx.set_global(ActionsRegistered);
    cx.on_action(|_: &Show, cx| {
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
