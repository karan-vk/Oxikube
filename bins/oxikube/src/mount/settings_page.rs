//! The settings page in the running app (E21-S04).
//!
//! `settings::Open` is a bus command (the palette's "Open Settings", an agent's
//! `app.settings_open`) whose handler asks the main window to open the page
//! (`bus::register_view_commands`, the same queue as `view::Open`); the window builds
//! `oxikube_settings_ui::page::SettingsPage` over the settings store (`ViewDeps::open`), once:
//! asking again shows the open tab.
//!
//! The keys reach it through actions: `cmd-,` / `ctrl-,` and the application menu's Settings…
//! run the `settings::Open` action, and a user keymap may still bind the former
//! `oxikube::OpenPreferences`. [`install_actions`] answers both by dispatching the command, so the
//! key, the menu, the palette and an agent run one behaviour.

use std::rc::{Rc, Weak};

use gpui::{Action, AnyWindowHandle, App};
use oxikube_domain::command::Command;
use oxikube_workspace::CommandDispatcher;
use oxikube_workspace::window::{OpenPreferences, settings};

/// Dispatches `settings::Open` when a key or the menu runs the `settings::Open` action (or the
/// former `oxikube::OpenPreferences`) while `window` is the active window.
///
/// The actions are app-wide (GPUI offers them to the app after the focused view declined them)
/// and every window installs this once, so each only answers while its own window is the active
/// one and passes the action on otherwise (see `keymap::install_action`). The handlers hold the
/// window's dispatcher weakly: a closed window's views and bus are not kept alive by them.
pub fn install_actions(
    window: AnyWindowHandle,
    dispatcher: &Rc<dyn CommandDispatcher>,
    cx: &mut App,
) {
    open_on::<settings::Open>(window, Rc::downgrade(dispatcher), cx);
    open_on::<OpenPreferences>(window, Rc::downgrade(dispatcher), cx);
}

fn open_on<A: Action>(
    window: AnyWindowHandle,
    dispatcher: Weak<dyn CommandDispatcher>,
    cx: &mut App,
) {
    cx.on_action(move |_: &A, cx| {
        if cx.active_window() == Some(window)
            && let Some(dispatcher) = dispatcher.upgrade()
        {
            dispatcher.dispatch(Command::SettingsOpen, cx);
        } else {
            cx.propagate();
        }
    });
}
