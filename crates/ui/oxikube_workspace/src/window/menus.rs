//! The application menu (`cx.set_menus`), its actions and their default key bindings.
//!
//! macOS shows the menu in the system menu bar. Linux and Windows have no global menu bar, so
//! `set_menus` is a no-op there and the same actions are reached by key binding (the in-window
//! menu bar arrives with the command palette, E11).
//!
//! The items are placeholders: Quit works, About opens a dialog, Preferences says it is not
//! there yet. Once the `CommandBus` and keymap core land (E05-S07, E11) they dispatch `Command`s
//! and the bindings move into `keymap.json`.

use crate::session::{NewWindow, Quit, ZoomIn, ZoomOut, ZoomReset};
use gpui::{
    App, KeyBinding, Menu, MenuItem, OsAction, ParentElement as _, SystemMenuType, actions,
};
use oxikube_ui::{
    dialog::{Dialog, OverlayExt as _, Toast},
    input::actions::{Copy, Cut, Paste, Redo, SelectAll, Undo},
};

actions!(
    oxikube,
    [
        /// Shows the About dialog.
        About,
        /// Opens the preferences (placeholder until the settings UI, E21).
        OpenPreferences,
        /// Hides the application (macOS).
        Hide,
        /// Hides every other application (macOS).
        HideOthers,
        /// Shows every application again (macOS).
        ShowAll,
        /// Minimises the active window.
        Minimize,
        /// Zooms (maximises or restores) the active window.
        Zoom,
    ]
);

/// The menu bar, in macOS order: application, Edit, View, Window.
pub fn app_menus() -> Vec<Menu> {
    vec![
        Menu::new("Oxikube").items([
            MenuItem::action("About Oxikube", About),
            MenuItem::separator(),
            MenuItem::action("Preferences…", OpenPreferences),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Oxikube", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Oxikube", Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::action("Actual Size", ZoomReset),
        ]),
        Menu::new("Window").items([
            MenuItem::action("New Window", NewWindow),
            MenuItem::separator(),
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]
}

/// Default key bindings of the menu actions (shown next to the items by macOS).
pub fn default_bindings(macos: bool) -> Vec<KeyBinding> {
    if macos {
        vec![
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", OpenPreferences, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
        ]
    } else {
        vec![KeyBinding::new("ctrl-q", Quit, None)]
    }
}

/// Registers the action handlers, key bindings and menu bar. Called once from [`super::init`].
pub(super) fn register(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| crate::session::request_quit(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window, _| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_active_window(cx, |window, _| window.zoom_window()));
    cx.on_action(|_: &About, cx| with_active_window(cx, |window, cx| show_about(window, cx)));
    cx.on_action(|_: &OpenPreferences, cx| {
        with_active_window(cx, |window, cx| {
            window.push_notification(
                Toast::new().message("Preferences are not available yet."),
                cx,
            )
        })
    });
    cx.bind_keys(default_bindings(cfg!(target_os = "macos")));
    cx.set_menus(app_menus());
}

/// Runs `f` on the active window after the current update ends. Menu actions reach the global
/// handlers while the window that dispatched them is still being updated, so touching that
/// window inline would fail; `defer` runs after the update has released it.
pub(crate) fn with_active_window(
    cx: &mut App,
    f: impl FnOnce(&mut gpui::Window, &mut App) + 'static,
) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| f(window, cx));
        }
    });
}

fn show_about(window: &mut gpui::Window, cx: &mut App) {
    window.open_dialog(cx, |dialog: Dialog, _, _| {
        dialog
            .title("About Oxikube")
            .child(format!("Oxikube {}", env!("CARGO_PKG_VERSION")))
    });
}
