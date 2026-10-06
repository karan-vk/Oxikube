//! The cluster context menu: read-only toggle and presets, as commands.
//!
//! [`cluster_menu_entries`] is the model (what the menu offers for a session, and the
//! [`Command`] each entry dispatches); [`cluster_menu`] fills a `PopupMenu` from it. The
//! cluster tab and hotbar entry use the same menu from their context-menu builders, so the
//! toggle exists once. Clicking an entry only builds a [`Command`] and hands it to the caller's
//! runner (normally [`ClusterCommandRunner::run`](super::ClusterCommandRunner::run)): the menu
//! never changes state itself, and the guard, not the menu, enforces what is allowed.

use std::rc::Rc;

use gpui::{App, Window};
use oxikube_app::ClusterSession;
use oxikube_domain::command::Command;
use oxikube_domain::{ClusterPreset, ids::ClusterId};
use oxikube_ui::menu::{PopupMenu, PopupMenuItem};

/// One row of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    /// The label.
    pub label: &'static str,
    /// Whether the row shows a check.
    pub checked: bool,
    /// What a click dispatches.
    pub command: Command,
}

/// The rows for `session`: "Read-only" (checked when on) and one row per preset, the current
/// preset checked.
pub fn cluster_menu_entries(session: &ClusterSession) -> Vec<MenuEntry> {
    entries(session.id(), session.read_only(), session.colour())
}

fn entries(
    cluster: &ClusterId,
    read_only: bool,
    colour: Option<oxikube_domain::ClusterColour>,
) -> Vec<MenuEntry> {
    let current = ClusterPreset::detect(colour);
    let mut rows = vec![MenuEntry {
        label: "Read-only",
        checked: read_only,
        command: Command::ClusterToggleReadOnly {
            cluster: cluster.clone(),
            read_only: Some(!read_only),
        },
    }];
    rows.extend(ClusterPreset::ALL.into_iter().map(|preset| MenuEntry {
        label: preset.label(),
        checked: preset == current,
        command: Command::ClusterApplyPreset {
            cluster: cluster.clone(),
            preset,
        },
    }));
    rows
}

/// Fills `menu` with the cluster rows. `run` receives the command of the clicked row.
pub fn cluster_menu(
    menu: PopupMenu,
    session: &ClusterSession,
    run: Rc<dyn Fn(Command, &mut Window, &mut App)>,
) -> PopupMenu {
    let mut rows = cluster_menu_entries(session).into_iter();
    let mut menu = menu;
    if let Some(toggle) = rows.next() {
        menu = menu.item(item(toggle, &run)).separator().label("Colour");
    }
    for row in rows {
        menu = menu.item(item(row, &run));
    }
    menu
}

fn item(row: MenuEntry, run: &Rc<dyn Fn(Command, &mut Window, &mut App)>) -> PopupMenuItem {
    let run = run.clone();
    let MenuEntry {
        label,
        checked,
        command,
    } = row;
    PopupMenuItem::new(label)
        .checked(checked)
        .on_click(move |_, window, cx| run(command.clone(), window, cx))
}
