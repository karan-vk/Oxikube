//! The cluster-tab actions: what `cmd-1..9` and the tab-cycling keys are bound to.
//!
//! Each action stands for a `Command` (`cluster::SwitchTab`, `cluster::NextTab`,
//! `cluster::PreviousTab`, same names) and does nothing but send it to the controller of the
//! window the key was pressed in, which applies it before the next frame. The bindings are in the per-OS keymap files of
//! `oxikube_assets` (`cmd-1` to `cmd-9` on macOS, `ctrl-shift-1` to `ctrl-shift-9` on Linux and
//! Windows, `ctrl-tab` and `ctrl-shift-tab` everywhere), so users rebind them in `keymap.json` like any
//! other key.

use gpui::{Action, App, Global, actions};
use oxikube_domain::command::Command;
use schemars::JsonSchema;
use serde::Deserialize;

use super::controller::TabsWindows;

actions!(
    cluster,
    [
        /// Show the next cluster tab (`cluster::NextTab`).
        NextTab,
        /// Show the previous cluster tab (`cluster::PreviousTab`).
        PreviousTab,
    ]
);

/// Show the nth cluster tab, counted from 1 (`cluster::SwitchTab`).
#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = cluster)]
pub struct SwitchTab {
    /// The tab, from 1 (the first cluster tab) to 9.
    pub index: u8,
}

/// Set once the handlers are installed, so a second `init` (a test app) adds none.
struct Registered;

impl Global for Registered {}

/// Routes the actions to the controller of the active window. Called by [`super::init`]; a
/// second call does nothing.
pub(super) fn register(cx: &mut App) {
    if cx.has_global::<Registered>() {
        return;
    }
    cx.set_global(Registered);
    cx.on_action(|_: &NextTab, cx| send(Command::ClusterNextTab, cx));
    cx.on_action(|_: &PreviousTab, cx| send(Command::ClusterPreviousTab, cx));
    cx.on_action(|action: &SwitchTab, cx| {
        send(
            Command::ClusterSwitchTab {
                index: action.index,
            },
            cx,
        )
    });
}

/// Queues `command` for the cluster tabs of the active window and applies it before this update
/// ends, so the frame after the key shows the other tab (E05-P600); nothing happens in a window
/// without them. The window is busy dispatching the key here, so the queue is applied when the
/// dispatch returns, still before the next frame.
fn send(command: Command, cx: &mut App) {
    let Some(window) = cx.active_window() else {
        return;
    };
    let Some(sink) = TabsWindows::sink(cx, window.window_id()) else {
        return;
    };
    if sink.send(command) {
        super::apply_before_next_frame(window, cx);
    }
}
