//! The `?` help overlay in the running app (E11-S10), over the real init path: the shipped keymap,
//! the real resource table and the real command bus.

use gpui::{Entity, TestAppContext};
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_palette::help::{HelpEntry, HelpOverlay, HelpState};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

impl App {
    fn help(&mut self) -> Option<Entity<HelpOverlay>> {
        let workspace = self.workspace();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<HelpOverlay>()
        })
    }

    fn help_entries(&mut self) -> Vec<HelpEntry> {
        let overlay = self.help().expect("the help overlay is open");
        self.vcx.update(|_, cx| {
            let picker = overlay.read(cx).picker().clone();
            picker.read(cx).delegate.model().entries().to_vec()
        })
    }

    fn table_has_focus(&mut self) -> bool {
        let ws = self.tab_workspace();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            gpui::Focusable::focus_handle(table.read(cx), cx).contains_focused(window, cx)
        })
    }
}

fn keys_of(entries: &[HelpEntry], action: &str) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.action == action)
        .map(HelpEntry::keystroke_text)
        .collect()
}

#[gpui::test]
fn question_mark_in_a_real_table_lists_its_keys(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    assert!(app.help().is_none());
    app.press("?");
    assert!(
        app.help().is_some(),
        "? opened the overlay in a cluster tab"
    );
    let entries = app.help_entries();
    assert_eq!(keys_of(&entries, "resource_table::ViewYaml"), ["y"]);
    assert!(keys_of(&entries, "resource_table::DeleteSelected").contains(&"ctrl-d".to_owned()));
    assert!(
        entries
            .iter()
            .all(|entry| entry.state == HelpState::Active || !entry.keystrokes.is_empty()),
        "every entry has keys"
    );
    assert!(
        keys_of(&entries, "log_view::ToggleWrap").is_empty(),
        "the log viewer's keys are not in force in a table"
    );

    // Escape closes it and the table has the focus again.
    app.press("escape");
    assert!(app.help().is_none());
    assert!(app.table_has_focus(), "focus returned to the table");
}

#[gpui::test]
fn in_the_tables_filter_field_question_mark_is_a_character(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    app.press("/");
    app.vcx.simulate_input("web?");
    app.vcx.run_until_parked();
    assert!(app.help().is_none(), "a text field takes the character");
}

#[gpui::test]
fn the_help_command_is_on_the_bus_with_a_tool_stub_and_opens_the_overlay(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_pods_table();
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    assert_eq!(bus.owner(CommandId::HELP_SHOW), Some("oxikube_palette"));
    assert!(bus.tool(CommandId::HELP_SHOW).is_some(), "an MCP tool stub");

    let outcome = futures::executor::block_on(bus.dispatch(
        Command::HelpShow,
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    app.vcx.run_until_parked();
    assert!(app.help().is_some(), "the bus command opened the overlay");
    // Asking again closes it: the same toggle the key has.
    let _ = futures::executor::block_on(bus.dispatch(
        Command::HelpShow,
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    app.vcx.run_until_parked();
    assert!(app.help().is_none());
}
