//! [`ThemeHost`]: the theme picker of one window, and how `theme::Select` reaches it.
//!
//! The binary builds one host per window when it mounts the cluster UI. The bus command (the
//! command palette, agents) queues a request on the [`ThemeSink`]; the window drains it on the UI
//! thread and toggles the picker.

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, Entity, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_theme::ThemeRegistry;
use oxikube_workspace::Workspace;

use super::delegate::ThemeDelegate;
use crate::picker::Picker;

/// A handle on a window's theme picker queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct ThemeSink {
    tx: UnboundedSender<()>,
}

impl ThemeSink {
    /// A sink and the receiver the window drains on the UI thread ([`ThemeHost::serve`]).
    pub fn channel() -> (Self, UnboundedReceiver<()>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }
}

/// Registers `theme::Select` on `registry` with its MCP tool stub:
/// `registry.install("oxikube_palette::theme", |r| register_commands(r, sink))`. The handler only
/// queues the request; the window shows the picker on the UI thread. Choosing a theme is a UI
/// preference, not a cluster mutation: no guard tier applies.
///
/// # Errors
///
/// A [`RegisterError`] when the id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: ThemeSink,
) -> Result<(), RegisterError> {
    let id = CommandId::THEME_SELECT;
    let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            if !matches!(command, Command::ThemeSelect) {
                return Err(OxiError::validation("not the theme::Select command"));
            }
            sink.tx
                .unbounded_send(())
                .map_err(|_| OxiError::internal("the window with the theme picker is gone"))?;
            Ok(CommandOutput::none())
        }
    })
}

/// The theme picker of one window. See the module docs.
pub struct ThemeHost {
    workspace: WeakEntity<Workspace>,
}

impl ThemeHost {
    /// The picker of the window whose main workspace is `workspace`.
    pub fn new(workspace: &Entity<Workspace>) -> Self {
        Self {
            workspace: workspace.downgrade(),
        }
    }

    /// Opens the picker, or closes it (reverting its preview) when it is open. Another modal that
    /// is open (a pending confirmation) is left alone: choosing a theme never takes the place of
    /// a decision. Does nothing before the theme registry is installed.
    pub fn toggle(&self, window: &mut Window, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let (open, other_modal) = {
            let layer = workspace.read(cx).modal_layer().read(cx);
            let open = layer.active_modal::<Picker<ThemeDelegate>>().is_some();
            (open, !open && layer.has_active_modal())
        };
        if open {
            workspace.update(cx, |workspace, cx| workspace.hide_modal(window, cx));
            return;
        }
        if other_modal || !cx.has_global::<ThemeRegistry>() {
            return;
        }
        let delegate = ThemeDelegate::new(cx);
        workspace.update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, move |window, cx| {
                Picker::uniform_list(delegate, window, cx)
            });
        });
    }

    /// Toggles the picker for each request that arrives on `requests` (the sink
    /// [`register_commands`] was given) in `window`, until the window closes. Hold the returned
    /// task as long as the window lives.
    pub fn serve(
        self: &std::rc::Rc<Self>,
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
