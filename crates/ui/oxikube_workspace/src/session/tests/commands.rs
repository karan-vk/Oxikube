//! Each session action is a registered `Command` with a tool stub, and has default key bindings
//! on every OS.

use gpui::{Action as _, TestAppContext};

use oxikube_domain::command::{CommandId, lookup, lookup_str};
use oxikube_keymap::{KeymapOptions, KeymapPlatform, KeymapStore};

use crate::session::{NewWindow, Quit, ZoomIn, ZoomOut, ZoomReset};

#[test]
fn actions_are_named_after_their_commands() {
    let pairs: [(&str, CommandId); 5] = [
        (Quit.name(), CommandId::APP_QUIT),
        (NewWindow.name(), CommandId::WINDOW_NEW),
        (ZoomIn.name(), CommandId::VIEW_ZOOM_IN),
        (ZoomOut.name(), CommandId::VIEW_ZOOM_OUT),
        (ZoomReset.name(), CommandId::VIEW_ZOOM_RESET),
    ];
    for (action, command) in pairs {
        assert_eq!(action, command.as_str());
        let meta = lookup(command).expect("registered command");
        assert!(!meta.mutating, "{command} is not a cluster mutation");
        // The MCP tool name is derived from the id: a stub exists as soon as the command does.
        assert!(command.tool_name().starts_with("app."), "{command}");
    }
    assert_eq!(CommandId::VIEW_ZOOM_IN.tool_name(), "app.view_zoom_in");
    assert_eq!(CommandId::WINDOW_NEW.tool_name(), "app.window_new");
    assert!(lookup_str("window::New").is_some());
}

#[gpui::test]
fn every_os_binds_zoom_new_window_and_quit(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
        for platform in [
            KeymapPlatform::MacOs,
            KeymapPlatform::Linux,
            KeymapPlatform::Windows,
        ] {
            let mut store = KeymapStore::new(KeymapOptions {
                platform,
                vim: false,
            });
            let merged = store.merge(cx);
            let bound = |action: &dyn gpui::Action| {
                merged
                    .bindings
                    .iter()
                    .any(|b| b.action().partial_eq(action))
            };
            assert!(bound(&Quit), "{platform:?}: Quit");
            assert!(bound(&NewWindow), "{platform:?}: NewWindow");
            assert!(bound(&ZoomIn), "{platform:?}: ZoomIn");
            assert!(bound(&ZoomOut), "{platform:?}: ZoomOut");
            assert!(bound(&ZoomReset), "{platform:?}: ZoomReset");
        }
    });
}

#[gpui::test]
fn the_zoom_keys_zoom(cx: &mut TestAppContext) {
    let _dir = super::setup(cx);
    cx.update(|cx| oxikube_keymap::init_with_text("[]", KeymapOptions::default(), cx));
    let (_handle, mut vcx) = super::open_window(cx);
    let (zoom_in, zoom_out, reset) = if cfg!(target_os = "macos") {
        ("cmd-=", "cmd--", "cmd-0")
    } else {
        ("ctrl-=", "ctrl--", "ctrl-0")
    };
    let scale = |vcx: &mut gpui::VisualTestContext| {
        vcx.update(|_, cx| oxikube_ui::UiScale::get(cx).factor())
    };
    vcx.simulate_keystrokes(zoom_in);
    assert_eq!(scale(&mut vcx), 1.1);
    vcx.simulate_keystrokes(zoom_out);
    vcx.simulate_keystrokes(zoom_out);
    assert_eq!(scale(&mut vcx), 0.9);
    vcx.simulate_keystrokes(reset);
    assert_eq!(scale(&mut vcx), 1.0);
}
