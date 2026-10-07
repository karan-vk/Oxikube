use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings as _, SettingsLocation, SettingsStore};

use super::*;
use crate::grid::{CursorShape, DEFAULT_SCROLLBACK_LINES, DefaultCursor, MAX_SCROLLBACK_LINES};

fn content(json: &str) -> TerminalContent {
    serde_json::from_str(json).expect("valid content")
}

fn resolve(json: &str) -> TerminalSettings {
    TerminalSettings::from_content(content(json))
}

fn cluster(n: u64) -> ClusterId {
    format!("{n:016x}").parse().expect("a cluster id")
}

fn store(user: &str) -> SettingsStore {
    let mut store = SettingsStore::new(oxikube_assets::default_settings()).expect("defaults");
    store.set_user_settings(user).expect("user settings");
    store
}

#[test]
fn defaults_leave_the_choice_to_the_environment_and_the_theme() {
    let settings = TerminalSettings::default();
    assert!(settings.shell.is_none() && settings.shell_args.is_empty());
    assert!(settings.font_family.is_none() && settings.font_size.is_none());
    assert_eq!(settings.line_height, 1.3);
    assert_eq!(settings.cursor_shape, CursorShapeSetting::Block);
    assert!(!settings.cursor_blink);
    assert_eq!(settings.bell, BellSetting::Visual);
    assert_eq!(
        settings.default_cursor(),
        DefaultCursor {
            shape: CursorShape::Block,
            blinking: false
        }
    );
}

#[test]
fn a_blank_shell_or_font_counts_as_unset() {
    let settings = resolve(r#"{"shell": "  ", "shell_args": ["-l"], "font_family": ""}"#);
    assert_eq!(settings.shell, None);
    assert_eq!(settings.shell_args, ["-l"]);
    assert_eq!(settings.font_family, None);
    let named = resolve(r#"{"shell": " /bin/zsh ", "font_family": " JetBrains Mono "}"#);
    assert_eq!(named.shell.as_deref(), Some("/bin/zsh"));
    assert_eq!(named.font_family.as_deref(), Some("JetBrains Mono"));
}

#[test]
fn scrollback_defaults_and_is_capped() {
    assert_eq!(
        TerminalSettings::default().scrollback_lines,
        DEFAULT_SCROLLBACK_LINES
    );
    assert_eq!(
        resolve(r#"{"scrollback_lines": 500}"#).scrollback_lines,
        500
    );
    assert_eq!(resolve(r#"{"scrollback_lines": 0}"#).scrollback_lines, 0);
    assert_eq!(
        resolve(r#"{"scrollback_lines": 5000000}"#).scrollback_lines,
        MAX_SCROLLBACK_LINES
    );
}

#[test]
fn font_and_line_height_are_clamped_to_their_ranges() {
    let small = resolve(r#"{"font_size": 1, "line_height": 0.2}"#);
    assert_eq!(small.font_size, Some(MIN_FONT_SIZE));
    assert_eq!(small.line_height, MIN_LINE_HEIGHT);
    let big = resolve(r#"{"font_size": 500, "line_height": 9}"#);
    assert_eq!(big.font_size, Some(MAX_FONT_SIZE));
    assert_eq!(big.line_height, MAX_LINE_HEIGHT);
    let fine = resolve(r#"{"font_size": 13.5, "line_height": 1.5}"#);
    assert_eq!((fine.font_size, fine.line_height), (Some(13.5), 1.5));
}

#[test]
fn a_font_size_that_is_not_a_number_falls_back_to_the_theme() {
    // JSON has no NaN, but a hand-built content can carry one.
    let nan = TerminalContent {
        font_size: Some(f32::NAN),
        line_height: Some(f32::INFINITY),
        ..TerminalContent::default()
    };
    let settings = TerminalSettings::from_content(nan);
    assert_eq!(settings.font_size, None);
    assert_eq!(
        settings.line_height,
        TerminalSettings::default().line_height
    );
}

#[test]
fn cursor_and_bell_choices_parse_by_name() {
    let settings = resolve(r#"{"cursor_shape": "bar", "cursor_blink": true, "bell": "audible"}"#);
    assert_eq!(settings.cursor_shape, CursorShapeSetting::Bar);
    assert_eq!(settings.bell, BellSetting::Audible);
    assert_eq!(
        settings.default_cursor(),
        DefaultCursor {
            shape: CursorShape::Beam,
            blinking: true
        }
    );
    assert_eq!(
        resolve(r#"{"cursor_shape": "underline", "bell": "none"}"#)
            .default_cursor()
            .shape,
        CursorShape::Underline
    );
    assert!(serde_json::from_str::<TerminalContent>(r#"{"cursor_shape": "hollow"}"#).is_err());
    assert!(serde_json::from_str::<TerminalContent>(r#"{"bell": "loud"}"#).is_err());
}

#[test]
fn the_exec_shell_chain_defaults_to_bash_then_sh_and_is_cleaned() {
    assert_eq!(TerminalSettings::default().exec_shells, ["bash", "sh"]);
    let content: TerminalContent =
        serde_json::from_str(r#"{"exec_shells": [" zsh ", "", "sh", "zsh"]}"#).unwrap();
    assert_eq!(
        TerminalSettings::from_content(content).exec_shells,
        ["zsh", "sh"]
    );
    let empty: TerminalContent = serde_json::from_str(r#"{"exec_shells": []}"#).unwrap();
    assert_eq!(
        TerminalSettings::from_content(empty).exec_shells,
        ["bash", "sh"],
        "an empty list falls back to the default instead of disabling shells"
    );
}

#[test]
fn the_input_settings_default_and_override() {
    let settings = TerminalSettings::default();
    assert!(!settings.copy_on_select);
    assert!(settings.confirm_multiline_paste);
    // Option composes characters on macOS, so meta is off there and on elsewhere.
    assert_eq!(settings.option_as_meta, !cfg!(target_os = "macos"));
    let settings = resolve(
        r#"{"copy_on_select": true, "option_as_meta": true, "confirm_multiline_paste": false}"#,
    );
    assert!(settings.copy_on_select && settings.option_as_meta);
    assert!(!settings.confirm_multiline_paste);
    assert!(!resolve(r#"{"option_as_meta": false}"#).option_as_meta);
}

#[test]
fn the_shipped_defaults_resolve_to_the_built_in_defaults() {
    let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
    let settings: &TerminalSettings = store.get(None);
    assert_eq!(settings, &TerminalSettings::default());
}

#[test]
fn a_cluster_overrides_the_user_who_overrides_the_defaults() {
    let (prod, dev) = (cluster(1), cluster(2));
    let user = format!(
        r#"{{
            "terminal": {{ "shell": "/bin/zsh", "font_size": 15, "bell": "none" }},
            "clusters": {{ "{prod}": {{ "terminal": {{ "shell": "/bin/bash", "shell_args": ["-l"] }} }} }}
        }}"#
    );
    let store = store(&user);
    let at = |id| Some(SettingsLocation { cluster: id });
    let global: &TerminalSettings = store.get(None);
    assert_eq!(global.shell.as_deref(), Some("/bin/zsh"));
    let prod: &TerminalSettings = store.get(at(&prod));
    assert_eq!(prod.shell.as_deref(), Some("/bin/bash"));
    assert_eq!(prod.shell_args, ["-l"]);
    // Everything the cluster does not say comes from the layer below.
    assert_eq!((prod.font_size, prod.bell), (Some(15.0), BellSetting::None));
    let dev: &TerminalSettings = store.get(at(&dev));
    assert_eq!(
        dev.shell.as_deref(),
        Some("/bin/zsh"),
        "no block: the global value"
    );
}

#[test]
fn the_schema_lists_the_terminal_keys_with_their_choices() {
    let store = SettingsStore::new(oxikube_assets::default_settings()).unwrap();
    let schema = store.json_schema();
    let properties = &schema["$defs"]["TerminalContent"]["properties"];
    for key in [
        "shell",
        "shell_args",
        "font_family",
        "font_size",
        "line_height",
        "scrollback_lines",
        "copy_on_select",
        "cursor_shape",
        "cursor_blink",
        "bell",
        "option_as_meta",
        "confirm_multiline_paste",
    ] {
        assert!(properties[key].is_object(), "`{key}` is in the schema");
        assert!(
            properties[key]["description"].is_string() || properties[key]["$ref"].is_string(),
            "`{key}` is described"
        );
    }
    assert_eq!(properties["font_size"]["minimum"], 6.0);
    assert_eq!(properties["font_size"]["maximum"], 72.0);
    let cursors = serde_json::to_string(&schema["$defs"]["CursorShapeSetting"]).unwrap();
    for choice in ["block", "bar", "underline"] {
        assert!(cursors.contains(choice), "{cursors}");
    }
    let bells = serde_json::to_string(&schema["$defs"]["BellSetting"]).unwrap();
    for choice in ["none", "visual", "audible"] {
        assert!(bells.contains(choice), "{bells}");
    }
}

#[test]
fn the_shipped_schema_file_matches_what_is_generated() {
    let shipped: serde_json::Value =
        serde_json::from_str(oxikube_assets::settings_schema()).expect("the shipped schema");
    let properties = &shipped["$defs"]["TerminalContent"]["properties"];
    for key in [
        "font_family",
        "font_size",
        "cursor_shape",
        "cursor_blink",
        "bell",
    ] {
        assert!(
            properties[key].is_object(),
            "`{key}` in settings.schema.json"
        );
    }
}
