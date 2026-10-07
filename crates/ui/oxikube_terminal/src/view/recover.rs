//! Recovering a terminal (E09-S12): [`TerminalView::reconnect`] opens a pod session again,
//! [`TerminalView::restart`] starts a fresh local shell. Both start a new process from the same
//! descriptor, so the launcher (and, for a pod, the exec service behind it) checks read-only mode
//! and the exec capability again: neither skips a policy a first open went through.
//!
//! The banner's buttons and the `terminal::Reconnect` / `terminal::Restart` / `terminal::Close`
//! commands both end here: a button focuses its terminal and sends the command (a command first, a
//! button second), and the window's `TerminalViews` applies it to the focused terminal.

use gpui::{Context, Window};
use oxikube_domain::command::Command;

use super::TerminalView;
use super::lifecycle::{Banner, BannerAction, Lifecycle, Signal};
use super::terminal_view::Phase;
use crate::element::TerminalElementState;

impl TerminalView {
    /// What the terminal is doing: starting, running, disconnected, exited, failed or closed.
    pub fn lifecycle(&self) -> &Lifecycle {
        &self.lifecycle
    }

    /// The banner the terminal shows above its screen, if its session dropped, ended or failed.
    pub fn banner(&self) -> Option<Banner> {
        self.lifecycle.banner(self.descriptor.is_local())
    }

    /// Feeds the lifecycle.
    pub(super) fn signal(&mut self, signal: Signal) {
        let local = self.descriptor.is_local();
        let state = std::mem::replace(&mut self.lifecycle, Lifecycle::Closed);
        self.lifecycle = state.apply(signal, local);
    }

    /// Stops typing from reaching a session that dropped (an ended one stops by itself).
    pub(super) fn stop_input_unless_running(&mut self, cx: &mut Context<Self>) {
        if self.lifecycle.accepts_input() {
            return;
        }
        if let Phase::Running(state) = &self.phase {
            state.update(cx, |state, _| state.close_input());
        }
    }

    /// Opens the session of a pod terminal again, after its connection dropped, it ended or it
    /// failed to start: a new session in the same container (the old shell's state is gone).
    /// Returns `false`, doing nothing, for a terminal that runs, or one that runs locally.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) -> bool {
        !self.descriptor.is_local() && self.relaunch(cx)
    }

    /// Starts a fresh shell in a local terminal whose shell exited (or could not start), with the
    /// same program, directory and cluster. Returns `false`, doing nothing, for a terminal that
    /// runs, or one that runs in a pod.
    pub fn restart(&mut self, cx: &mut Context<Self>) -> bool {
        self.descriptor.is_local() && self.relaunch(cx)
    }

    fn relaunch(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.lifecycle.can_relaunch() {
            return false;
        }
        if let Phase::Running(state) = &self.phase {
            // The old session is dead or dying; ending it again is idempotent and makes sure a
            // half-open connection is torn down. The pump and writer are aborted when the state
            // is dropped just below.
            state.update(cx, |state, cx| state.kill(cx)).detach();
        }
        self.phase = Phase::Starting;
        self.subscriptions.clear();
        self.process_title = None;
        // The old screen's row cache and size request belong to the old grid.
        self.element = TerminalElementState::new();
        self.signal(Signal::Relaunch);
        // Replacing the slot drops the previous launch, which finished long ago, from outside it.
        self.launch = Some(Self::start(&self.descriptor, &self.services, cx));
        cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
        cx.notify();
        true
    }

    /// A banner button: focuses this terminal and sends the command, which `TerminalViews`
    /// applies to the focused terminal. Without a dispatcher (a bare view) it acts directly.
    pub(super) fn perform(
        &mut self,
        action: BannerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let command = match action {
            BannerAction::Reconnect => Command::TerminalReconnect,
            BannerAction::Restart => Command::TerminalRestart,
            BannerAction::CloseTab => Command::TerminalClose,
        };
        if self.services.dispatcher().is_some() {
            let services = self.services.clone();
            // After this update: the command may come back and update this terminal.
            cx.defer(move |cx| services.dispatch(command, cx));
            return;
        }
        match action {
            BannerAction::Reconnect => {
                self.reconnect(cx);
            }
            BannerAction::Restart => {
                self.restart(cx);
            }
            // Closing is the workspace's: it needs the pane that holds the tab.
            BannerAction::CloseTab => {}
        }
    }
}
