//! The view's operations and the requests that ask for them.
//!
//! A key, a toolbar button, the palette and an agent all go through the bus: `request_*` sends
//! the `logs::*` command with the view's target, the bus hands it to the window's
//! [`LogViews`](crate::LogViews), which calls the operation (`set_range`, `toggle_wrap`, ...) on
//! the views of that target. Nothing here changes a cluster: these commands read logs or change
//! what a view shows, so no `MutationGuard` tier applies.

use gpui::Context;
use oxikube_domain::command::Command;
use oxikube_domain::log::LogRange;

use super::LogView;

impl LogView {
    /// Reads `range` of the log (reopens the stream when it changes).
    pub fn set_range(&mut self, range: LogRange, cx: &mut Context<Self>) {
        if self.options.range != range {
            self.options.range = range;
            self.open_stream(cx);
        }
    }

    /// Reads `container` (reopens the stream when it changes).
    pub fn select_container(&mut self, container: &str, cx: &mut Context<Self>) {
        if self.options.container.as_deref() != Some(container) {
            self.options.container = Some(container.to_owned());
            cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
            self.open_stream(cx);
        }
    }

    /// Reads the previous container instance, or the current one (reopens the stream).
    pub fn toggle_previous(&mut self, cx: &mut Context<Self>) {
        self.options.previous = !self.options.previous;
        self.open_stream(cx);
    }

    /// Wraps long lines, or not; the line at the top of the screen stays there.
    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.switch_wrap(!self.options.wrap, cx);
    }

    /// Shows the timestamps, or not.
    pub fn toggle_timestamps(&mut self, cx: &mut Context<Self>) {
        self.options.timestamps = !self.options.timestamps;
        if self.options.wrap {
            // The rows' heights may change with their text.
            self.list.remeasure();
        }
        cx.notify();
    }

    /// Follows the newest line (jumping to it), or stops following.
    pub fn toggle_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.follow.is_on() {
            self.pause(cx);
        } else {
            self.resume(cx);
        }
    }

    /// Lets the view fill its cluster tab (the pane is zoomed, the docks hidden), or gives the
    /// space back.
    pub fn toggle_fullscreen(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        // The view's pane is the active one: the key or the click that asked came from it.
        workspace.update(cx, |ws, cx| ws.toggle_zoom(window, cx));
        cx.notify();
    }

    /// Whether the view's workspace has a pane zoomed (fullscreen).
    pub fn is_fullscreen(&self, cx: &gpui::App) -> bool {
        self.workspace
            .as_ref()
            .and_then(|w| w.upgrade())
            .is_some_and(|ws| ws.read(cx).is_zoomed(cx))
    }

    fn send(&self, command: Command, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Asks for `range` (`logs::SetRange`).
    pub fn request_range(&mut self, range: LogRange, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsSetRange { target, range }, cx);
    }

    /// Asks for `container` (`logs::SelectContainer`).
    pub fn request_container(&mut self, container: &str, cx: &mut Context<Self>) {
        let target = self.target.clone();
        let container = container.to_owned();
        self.send(Command::LogsSelectContainer { target, container }, cx);
    }

    /// Asks to toggle the previous instance (`logs::TogglePrevious`).
    pub fn request_previous(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsTogglePrevious { target }, cx);
    }

    /// Asks to toggle the wrap (`logs::ToggleWrap`).
    pub fn request_wrap(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsToggleWrap { target }, cx);
    }

    /// Asks to toggle the timestamps (`logs::ToggleTimestamps`).
    pub fn request_timestamps(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsToggleTimestamps { target }, cx);
    }

    /// Asks to toggle autoscroll (`logs::ToggleAutoscroll`).
    pub fn request_autoscroll(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsToggleAutoscroll { target }, cx);
    }

    /// Asks to toggle fullscreen (`logs::ToggleFullscreen`).
    pub fn request_fullscreen(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsToggleFullscreen { target }, cx);
    }

    /// The pill: follows again, as `logs::ToggleAutoscroll` does while autoscroll is off.
    pub fn jump_to_newest(&mut self, cx: &mut Context<Self>) {
        if !self.follow.is_on() {
            self.request_autoscroll(cx);
        }
    }

    /// `m`: marks are E08-S06's. The key is bound so it stays reserved.
    pub(crate) fn mark(&mut self, _: &mut Context<Self>) {
        tracing::debug!(target = %self.target, "log marks arrive with E08-S06");
    }

    /// `c`: copying lines is E08-S06's. The key is bound so it stays reserved.
    pub(crate) fn copy(&mut self, _: &mut Context<Self>) {
        tracing::debug!(target = %self.target, "copying log lines arrives with E08-S06");
    }
}
