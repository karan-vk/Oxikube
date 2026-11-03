//! The theme picker in the running app (E22-S08), over the real init path: the command palette
//! lists `theme::Select`, running it opens the picker through the bus, arrows preview, Escape
//! reverts and Enter writes `settings.json`.

use gpui::{Entity, TestAppContext, UpdateGlobal as _};
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_palette::picker::Picker;
use oxikube_palette::theme_picker::ThemeDelegate;
use oxikube_settings::SettingsStore;
use oxikube_testkit::TestPorts;
use oxikube_theme::ActiveTheme;

use super::App;
use crate::app_state::AppState;

const OPEN_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-shift-p"
} else {
    "ctrl-shift-p"
};

const SETTINGS: &str = "{\n  // mine\n  \"theme\": { \"mode\": \"dark\", \"light\": \"One Light\", \"dark\": \"One Dark\" }\n}\n";

/// The app over in-memory settings holding [`SETTINGS`], with a cluster tab open.
fn start(cx: &mut TestAppContext) -> App {
    let mut app = App::start_with(cx, TestPorts::seeded(), |cx| {
        SettingsStore::update_global(cx, |store, _| {
            store.set_user_settings(SETTINGS).expect("settings parse");
        });
    });
    app.open_pods_table();
    app
}

fn settings_text(app: &mut App) -> String {
    app.vcx.update(|_, cx| {
        cx.global::<SettingsStore>()
            .user_settings_text()
            .unwrap_or_default()
            .to_owned()
    })
}

impl App {
    fn theme_picker(&mut self) -> Option<Entity<Picker<ThemeDelegate>>> {
        let workspace = self.workspace();
        self.vcx.update(|_, cx| {
            workspace
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<Picker<ThemeDelegate>>()
        })
    }

    fn active_theme(&mut self) -> String {
        self.vcx.update(|_, cx| ActiveTheme::get(cx).name.clone())
    }
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub_and_opens_the_picker(cx: &mut TestAppContext) {
    let mut app = start(cx);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    assert_eq!(
        bus.owner(CommandId::THEME_SELECT),
        Some("oxikube_palette::theme")
    );
    assert!(
        bus.tool(CommandId::THEME_SELECT).is_some(),
        "an MCP tool stub"
    );

    let outcome = futures::executor::block_on(bus.dispatch(
        Command::ThemeSelect,
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    assert!(outcome.is_ok(), "{outcome:?}");
    app.vcx.run_until_parked();
    assert!(app.theme_picker().is_some(), "the bus command opened it");
}

#[gpui::test]
fn select_theme_from_the_palette_previews_reverts_and_commits(cx: &mut TestAppContext) {
    let mut app = start(cx);
    assert_eq!(app.active_theme(), "One Dark");

    app.press(OPEN_KEY);
    app.vcx.simulate_input("select theme");
    app.vcx.run_until_parked();
    app.press("enter");
    app.vcx.run_until_parked();
    assert!(
        app.theme_picker().is_some(),
        "Select Theme in the palette opens the picker"
    );

    // Down from One Dark lands on the next bundled dark theme, whichever the set holds.
    app.press("down");
    let next = app.active_theme();
    assert_ne!(next, "One Dark", "previewed");
    app.press("escape");
    assert!(app.theme_picker().is_none());
    assert_eq!(app.active_theme(), "One Dark", "Escape reverted");

    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let _ = futures::executor::block_on(bus.dispatch(
        Command::ThemeSelect,
        DispatchContext::new(Initiator::Agent, "agent"),
    ));
    app.vcx.run_until_parked();
    app.press("down");
    app.press("enter");
    app.vcx.run_until_parked();
    let text = settings_text(&mut app);
    assert!(text.contains("// mine"), "{text}");
    assert!(text.contains(&format!("\"dark\": \"{next}\"")), "{text}");
    assert_eq!(app.active_theme(), next);
}
