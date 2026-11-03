//! The keymap editor in the running app (E21-S07): `keymap::ShowEditor` on the bus with its tool
//! stub, the sidebar's Settings > Keymap entry opening the page in the connected cluster's tab,
//! and the page's "Open keymap.json" running `keymap::OpenUser` on the bus.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_settings_ui::keymap::KeymapEditorView;
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;

fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

fn pages(app: &mut App) -> usize {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<KeymapEditorView>()
            .len()
    })
}

#[gpui::test]
fn the_command_is_on_the_bus_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    assert_eq!(
        bus.owner(CommandId::KEYMAP_SHOW_EDITOR),
        Some("oxikube_settings_ui::keymap")
    );
    assert_eq!(
        bus.tool(CommandId::KEYMAP_SHOW_EDITOR)
            .map(|t| t.name.as_str()),
        Some("app.keymap_show_editor")
    );
}

#[gpui::test]
fn the_sidebar_entry_and_the_command_open_one_page(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    assert!(
        app.drawn("sidebar-section-settings"),
        "the Settings section is in the sidebar"
    );
    assert_eq!(pages(&mut app), 0, "nothing opens by itself");

    app.click("sidebar-entry-settings/keymap");
    assert_eq!(pages(&mut app), 1, "one page in the cluster's tab");
    assert!(app.drawn("keymap-editor"));
    assert!(app.drawn("keymap-table"));

    // Asking again (the palette's command) shows the open page.
    assert!(run(&mut app, Command::KeymapShowEditor));
    assert_eq!(pages(&mut app), 1);
}

/// The keymap editing commands (E21-S08) are on the bus with tool stubs, and a
/// `keymap::SetBinding` from the palette or an agent writes the user's `keymap.json` and is in
/// force at once; an invalid one is refused and writes nothing.
#[gpui::test]
fn set_and_remove_binding_write_the_user_keymap_and_apply_at_once(cx: &mut TestAppContext) {
    use oxikube_domain::command::{KeymapBindingEdit, KeymapBindingRef, KeymapSource};

    use crate::startup::ConfigSource;

    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, "// mine\n[]\n").expect("the user keymap");
    let config = dir.path().to_owned();
    let mut app = App::start_with_env(
        cx,
        TestPorts::seeded(),
        move |env| env.config = ConfigSource::Dir(config),
        |_| {},
    );
    // Connect the seeded cluster, so there is a tab for the page.
    app.press("enter");
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    for (id, tool) in [
        (
            CommandId::KEYMAP_RECORD_BINDING,
            "app.keymap_record_binding",
        ),
        (CommandId::KEYMAP_SET_BINDING, "app.keymap_set_binding"),
        (
            CommandId::KEYMAP_REMOVE_BINDING,
            "app.keymap_remove_binding",
        ),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_settings_ui::keymap"));
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool));
    }

    let set = |action: &str| Command::KeymapSetBinding {
        edit: KeymapBindingEdit {
            action: action.to_owned(),
            data: None,
            context: None,
            keystrokes: "ctrl-alt-shift-e".into(),
            replaces: None,
            unbind: Vec::new(),
        },
    };
    let dispatch = |app: &mut App, command: Command| {
        let bus = bus.clone();
        let task = app.vcx.executor().spawn(async move {
            bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me"))
                .await
        });
        app.vcx.run_until_parked();
        block_on(task)
    };
    assert!(dispatch(&mut app, set("nope::Missing")).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).expect("the file"),
        "// mine\n[]\n"
    );

    assert!(dispatch(&mut app, set("keymap::ShowEditor")).is_ok());
    let text = std::fs::read_to_string(&path).expect("the file");
    assert!(text.starts_with("// mine"), "{text}");
    assert!(
        text.contains("\"ctrl-alt-shift-e\": \"keymap::ShowEditor\""),
        "{text}"
    );
    assert_eq!(pages(&mut app), 0);
    // The action handlers answer for the active window only.
    app.vcx.update(|window, _| window.activate_window());
    app.vcx.run_until_parked();
    let resolved = app.vcx.update(|window, cx| {
        oxikube_keymap::resolve(cx, "ctrl-alt-shift-e", &window.context_stack())
            .ok()
            .and_then(|resolution| resolution.action())
    });
    assert_eq!(resolved, Some("keymap::ShowEditor"));
    app.press("ctrl-alt-shift-e");
    assert_eq!(
        pages(&mut app),
        1,
        "the new key opens the page without a restart"
    );

    let remove = Command::KeymapRemoveBinding {
        binding: KeymapBindingRef {
            context: None,
            keystrokes: "ctrl-alt-shift-e".into(),
            source: KeymapSource::User,
        },
    };
    assert!(dispatch(&mut app, remove).is_ok());
    let text = std::fs::read_to_string(&path).expect("the file");
    assert!(!text.contains("ctrl-alt-shift-e"), "{text}");
}
