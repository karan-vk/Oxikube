//! Scenario A: the command palette, opened with its key, filtered by typing, confirmed with Enter
//! (E11 done-when 1). What it runs reaches the real `CommandBus`, its `MutationGuard` and audit.

use std::collections::BTreeSet;

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_testkit::TestPorts;

use super::{OPEN_PALETTE, check};
use crate::app_state::AppState;
use crate::mount::tests::App;

/// The key that lists the unavailable commands too (the shipped keymap's).
const SHOW_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-shift-a"
} else {
    "ctrl-shift-a"
};

impl App {
    fn palette_listed(&mut self) -> Vec<CommandId> {
        let palette = self.palette_open().expect("the palette is open");
        self.vcx
            .update(|_, cx| palette.read(cx).picker().read(cx).delegate.listed())
    }

    fn palette_selected(&mut self) -> Option<CommandId> {
        let palette = self.palette_open().expect("the palette is open");
        self.vcx
            .update(|_, cx| palette.read(cx).picker().read(cx).delegate.selected_id())
    }

    fn read_only(&mut self) -> bool {
        self.vcx.update(|_, cx| {
            AppState::global(cx)
                .services()
                .sessions
                .is_read_only(&TestPorts::cluster_id())
        })
    }

    /// Opens the palette, types `query` and confirms the selected row.
    fn run_from_palette(&mut self, query: &str, expect: CommandId) {
        self.press(OPEN_PALETTE);
        check!(
            self,
            self.palette_open().is_some(),
            "{OPEN_PALETTE} opens the palette"
        );
        self.type_text(query);
        let selected = self.palette_selected();
        check!(
            self,
            selected == Some(expect),
            "{query:?} selects {expect}, not {selected:?}"
        );
        self.press("enter");
        self.tick();
        self.tick();
        check!(
            self,
            self.palette_open().is_none(),
            "confirming closes the palette"
        );
    }
}

#[gpui::test]
fn open_type_confirm_runs_the_command_through_the_bus_and_the_audit_log(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    assert!(!app.read_only(), "the session starts writable");

    app.run_from_palette("toggle read-only", CommandId::CLUSTER_TOGGLE_READ_ONLY);

    check!(
        app,
        app.read_only(),
        "the palette's command reached its handler"
    );
    let audit = app.ports.state.audit_log();
    check!(app, audit.len() == 1, "one record, not {audit:?}");
    assert_eq!(&*audit[0].cmd, "cluster::ToggleReadOnly");
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    let recent = app
        .vcx
        .update(|_, cx| AppState::global(cx).recents().recent());
    assert_eq!(
        recent,
        [CommandId::CLUSTER_TOGGLE_READ_ONLY],
        "and is a recent"
    );
}

#[gpui::test]
fn a_read_only_session_hides_mutations_until_the_palette_lifts_it(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    app.press("down");
    app.press(OPEN_PALETTE);
    app.type_text("delete");
    let before = app.palette_listed();
    check!(
        app,
        before.contains(&CommandId::RESOURCE_DELETE),
        "a writable cluster offers Delete: {before:?}"
    );
    app.press("escape");

    app.run_from_palette("toggle read-only", CommandId::CLUSTER_TOGGLE_READ_ONLY);
    assert!(app.read_only());

    app.press(OPEN_PALETTE);
    app.type_text("delete");
    let hidden = app.palette_listed();
    check!(
        app,
        !hidden.contains(&CommandId::RESOURCE_DELETE),
        "read-only hides Delete (hidden, not greyed): {hidden:?}"
    );
    // "Show all" lists it, marked, and confirming it does nothing.
    app.press(SHOW_ALL);
    let all = app.palette_listed();
    check!(
        app,
        all.contains(&CommandId::RESOURCE_DELETE),
        "show all lists it"
    );
    app.press("escape");

    // Lifting read-only is itself allowed in a read-only session (the escape hatch).
    app.run_from_palette("toggle read-only", CommandId::CLUSTER_TOGGLE_READ_ONLY);
    check!(app, !app.read_only(), "the palette lifted read-only again");
    app.press(OPEN_PALETTE);
    app.type_text("delete");
    let again = app.palette_listed();
    check!(
        app,
        again.contains(&CommandId::RESOURCE_DELETE),
        "Delete is back: {again:?}"
    );
}

#[gpui::test]
fn escape_closes_the_palette_and_the_table_has_its_keys_back(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    app.press(OPEN_PALETTE);
    check!(app, app.palette_open().is_some(), "opened");
    app.press("escape");
    check!(app, app.palette_open().is_none(), "escape closes it");
    assert!(app.cursor().is_none(), "nothing ran");
    app.press("down");
    let cursor = app.cursor();
    check!(
        app,
        cursor.is_some(),
        "the table got the focus back: down moved its cursor"
    );
    assert!(app.ports.state.audit_log().is_empty());
}

#[gpui::test]
fn delete_from_the_palette_opens_the_tables_confirmation_and_deletes_nothing(
    cx: &mut TestAppContext,
) {
    let mut app = App::keyboard(cx, false);
    app.press("down");
    let target = app.cursor().expect("a row under the cursor");
    let resources = app
        .ports
        .connector
        .ports_for(&TestPorts::cluster_id())
        .resources;

    app.press(OPEN_PALETTE);
    app.type_text("resource delete");
    let selected = app.palette_selected();
    check!(
        app,
        selected == Some(CommandId::RESOURCE_DELETE),
        "the query finds Delete first, not {selected:?}"
    );
    app.press("enter");
    app.tick();

    check!(
        app,
        app.delete_dialog_open().is_some(),
        "the palette hands Delete to the table's own flow: the one confirmation dialog"
    );
    let dialog = app.delete_dialog_open().expect("open");
    let planned = app
        .vcx
        .update(|_, cx| dialog.read(cx).plan().items()[0].target.clone());
    assert_eq!(planned, target, "it acts on the row the cursor was on");
    assert!(
        resources.mutating_calls().is_empty(),
        "the palette never confirms for anyone"
    );
    assert!(app.ports.state.audit_log().is_empty());
}

#[gpui::test]
fn show_all_lists_every_command_the_bus_registered_with_its_binding(cx: &mut TestAppContext) {
    let mut app = App::keyboard(cx, false);
    let bus = app
        .vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the mount set the bus");

    app.press(OPEN_PALETTE);
    app.press(SHOW_ALL);
    let listed: BTreeSet<CommandId> = app.palette_listed().into_iter().collect();
    let registered: BTreeSet<CommandId> = bus.all().iter().map(|info| info.id()).collect();
    check!(
        app,
        listed == registered,
        "missing from the palette: {:?}; not on the bus: {:?}",
        registered.difference(&listed).collect::<Vec<_>>(),
        listed.difference(&registered).collect::<Vec<_>>()
    );

    // A bound command is drawn with its key caps: the palette's own command is bound to the key
    // that opened it.
    app.type_text("toggle command palette");
    let selected = app.palette_selected();
    check!(
        app,
        selected == Some(CommandId::PALETTE_TOGGLE),
        "selected {selected:?}"
    );
    let bound = app
        .vcx
        .update(|_, cx| oxikube_keymap::bindings_for_command(cx, CommandId::PALETTE_TOGGLE));
    check!(app, !bound.is_empty(), "the keymap binds palette::Toggle");
    check!(
        app,
        app.drawn("palette-keys-0"),
        "and its row shows the key caps"
    );
}
