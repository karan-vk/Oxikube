//! The `LogView` keymap as data (E08-S10): the shipped defaults, every platform alike, resolve to
//! the view's actions, and a user's `keymap.json` rebinds or unbinds them.

use gpui::TestAppContext;
use oxikube_assets::{KeymapPlatform, default_keymap};
use oxikube_domain::command::Command;
use oxikube_keymap::file::parse_keymap;
use oxikube_keymap::{ActionRegistry, KeymapAction, KeymapLayer, bindings_for_action_name};

use super::fixture::{Fx, lines, pod_ref};
use oxikube_testkit::Timeline;

/// The context of the log view's bare keys: off while a text field inside the view has focus.
const CONTEXT: &str = "LogView && !Editing";

/// k9s's keys, as the shipped keymap binds them.
const DEFAULTS: [(&str, &str); 18] = [
    ("0", "log_view::Tail"),
    ("1", "log_view::Head"),
    ("2", "log_view::Since1m"),
    ("3", "log_view::Since5m"),
    ("4", "log_view::Since15m"),
    ("5", "log_view::Since30m"),
    ("6", "log_view::Since1h"),
    ("s", "log_view::ToggleAutoscroll"),
    ("w", "log_view::ToggleWrap"),
    ("t", "log_view::ToggleTimestamps"),
    ("j", "log_view::ToggleJsonMode"),
    ("p", "log_view::TogglePrevious"),
    ("f", "log_view::ToggleFullscreen"),
    ("m", "log_view::Mark"),
    ("c", "log_view::Copy"),
    ("/", "log_view::Find"),
    ("n", "log_view::NextMatch"),
    ("shift-n", "log_view::PreviousMatch"),
];

fn defaults_of(platform: KeymapPlatform) -> Vec<(String, String)> {
    let parsed = parse_keymap(default_keymap(platform), KeymapLayer::Default).expect("a keymap");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let sections: Vec<_> = parsed
        .sections
        .iter()
        .filter(|(_, section)| section.context_expr() == Some(CONTEXT))
        .collect();
    assert_eq!(sections.len(), 1, "one `{CONTEXT}` section on {platform:?}");
    sections[0]
        .1
        .bindings
        .iter()
        .map(
            |(key, value)| match KeymapAction::from_json(value).expect("a binding") {
                KeymapAction::Action { name, data: None } => (key.clone(), name),
                other => panic!("{key}: {other:?}"),
            },
        )
        .collect()
}

#[test]
fn every_platform_ships_the_same_log_view_keys() {
    let expected: Vec<(String, String)> = DEFAULTS
        .iter()
        .map(|(key, name)| ((*key).to_owned(), (*name).to_owned()))
        .collect();
    for platform in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Linux,
        KeymapPlatform::Windows,
    ] {
        assert_eq!(defaults_of(platform), expected, "{platform:?}");
    }
}

#[gpui::test]
fn the_default_keys_name_actions_the_view_handles(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    fx.draw();
    let focus = fx.read(&view, |v| v.focus.clone());
    for (key, name) in DEFAULTS {
        fx.vcx.update(|window, cx| {
            assert!(
                ActionRegistry::from_app(cx).contains(name),
                "{name} is registered"
            );
            let infos = bindings_for_action_name(cx, name, None);
            let info = infos
                .iter()
                .find(|info| info.keystrokes_text() == key)
                .unwrap_or_else(|| panic!("{key} is bound to {name}: {infos:?}"));
            assert_eq!(info.context.as_deref(), Some(CONTEXT));
            assert_eq!(info.layer, Some(KeymapLayer::Default));
            let action = cx.build_action(name, None).expect("the action builds");
            assert!(
                window.is_action_available_in(action.as_ref(), &focus),
                "the view handles {name}"
            );
        });
    }
}

fn rebind(fx: &mut Fx, user_keymap: &str) {
    let text = user_keymap.to_owned();
    fx.vcx
        .update(|_, cx| oxikube_keymap::reload_user_keymap(cx, &text));
    fx.settle();
}

#[gpui::test]
fn a_users_keymap_rebinds_and_unbinds_the_log_view_keys(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    let target = pod_ref();

    rebind(
        &mut fx,
        r#"[{ "context": "LogView && !Editing",
              "bindings": { "w": null, "x": "log_view::ToggleWrap",
                            "ctrl-t": "log_view::ToggleTimestamps" } }]"#,
    );
    // The palette's hint follows: `x` leads, `w` is gone.
    fx.vcx.update(|_, cx| {
        let infos = bindings_for_action_name(cx, "log_view::ToggleWrap", None);
        let keys: Vec<_> = infos.iter().map(|i| i.keystrokes_text()).collect();
        assert_eq!(keys, ["x"], "{infos:?}");
        assert_eq!(infos[0].layer, Some(KeymapLayer::User));
    });

    fx.keys("w");
    assert!(!fx.read(&view, |v| v.options().wrap), "w is unbound");
    assert!(fx.dispatcher.sent().is_empty(), "and sends nothing");
    fx.keys("x");
    assert!(fx.read(&view, |v| v.options().wrap), "x toggles the wrap");
    fx.keys("ctrl-t");
    assert!(fx.read(&view, |v| v.options().timestamps));
    // A default the user left alone still works.
    fx.keys("s");
    assert!(!fx.read(&view, |v| v.autoscroll()));
    assert_eq!(
        fx.dispatcher.sent(),
        [
            Command::LogsToggleWrap {
                target: target.clone()
            },
            Command::LogsToggleTimestamps {
                target: target.clone()
            },
            Command::LogsToggleAutoscroll { target },
        ]
    );

    // Emptying the file brings the defaults back.
    rebind(&mut fx, "[]");
    fx.keys("w");
    assert!(
        !fx.read(&view, |v| v.options().wrap),
        "w toggles it off again"
    );
}

#[gpui::test]
fn a_broken_user_keymap_leaves_the_defaults_in_place(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    rebind(&mut fx, "{ not a keymap");
    fx.keys("w");
    assert!(fx.read(&view, |v| v.options().wrap));
}
