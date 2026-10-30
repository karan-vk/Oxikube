//! The settings schema generator: `oxikube --print-settings-schema` / `--print-settings-crates`.
//!
//! `settings.schema.json` is built from the settings that `inventory` collected, and only crates
//! linked into the running binary contribute. This binary links every crate the app starts, so it
//! is the one target whose schema covers every setting a user can write (E05-S06b).
//! `cargo xtask gen-settings-schema` runs it, and cross-checks the crates reported by
//! `--print-settings-crates` against the crates whose source invokes `register_settings!`, so a
//! settings crate nobody links fails loudly instead of vanishing from the schema.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::process::ExitCode;

use oxikube_keymap::BaseKeymap;
use oxikube_settings::schema::to_schema_text;
use oxikube_settings::schema_dynamic;
use oxikube_settings::{DynamicEnums, RegisteredSetting, SettingsStore};
use oxikube_theme::ThemeRegistry;

use crate::cli::Print;

/// Runs a [`Print`] request and returns the exit code.
pub fn print(what: Print) -> ExitCode {
    let text = match what {
        Print::SettingsSchema => schema_text().map_err(|err| err.to_string()),
        Print::LiveSettingsSchema => live_schema_text().map_err(|err| err.to_string()),
        Print::SettingsCrates => Ok(crates_text()),
    };
    match text {
        Ok(text) => {
            // A closed pipe (the reader went away) is not worth a panic.
            let _ = std::io::stdout().lock().write_all(text.as_bytes());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("oxikube: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The text of `settings.schema.json` for every setting registered in this binary.
pub fn schema_text() -> oxikube_domain::OxiResult<String> {
    let store = SettingsStore::new(oxikube_assets::default_settings())?;
    Ok(to_schema_text(&store.json_schema()))
}

/// The dynamic enum lists known without a window: the bundled themes and the base keymaps. The
/// running app adds the user themes and the installed fonts (see [`live_enums`]).
pub fn bundled_enums() -> DynamicEnums {
    live_enums(&ThemeRegistry::with_bundled(), Vec::new())
}

/// The lists the in-app JSON editor completes from: `registry`'s theme names, every
/// [`BaseKeymap`] and `fonts` (the installed families, from the text system).
pub fn live_enums(registry: &ThemeRegistry, fonts: Vec<String>) -> DynamicEnums {
    DynamicEnums::new()
        .with(schema_dynamic::THEMES, registry.names())
        .with(schema_dynamic::BASE_KEYMAPS, BaseKeymap::names())
        .with(schema_dynamic::FONTS, fonts)
}

/// The schema text with [`bundled_enums`] overlaid (`--print-live-settings-schema`).
pub fn live_schema_text() -> oxikube_domain::OxiResult<String> {
    let store = SettingsStore::new(oxikube_assets::default_settings())?;
    Ok(to_schema_text(&store.json_schema_with(&bundled_enums())))
}

/// The distinct crates that registered a setting, sorted, one per line.
pub fn crates_text() -> String {
    let crates: BTreeSet<&str> = RegisteredSetting::all()
        .map(RegisteredSetting::crate_name)
        .collect();
    crates.into_iter().map(|name| format!("{name}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_covers_the_settings_of_every_linked_crate() {
        let schema: serde_json::Value = serde_json::from_str(&schema_text().unwrap()).unwrap();
        let properties = schema["properties"].as_object().unwrap();
        let registered: Vec<_> = RegisteredSetting::all().collect();
        assert!(!registered.is_empty());
        for setting in registered {
            if let Some(key) = setting.key() {
                assert!(
                    properties.contains_key(key),
                    "`{key}` (registered by {}) is missing from the schema",
                    setting.crate_name()
                );
            }
        }
    }

    /// E21-S02: the live schema injects the bundled theme names and the base keymaps, while the
    /// static (checked-in) schema carries only the annotations and stays deterministic.
    #[test]
    fn the_live_schema_injects_theme_and_keymap_names_and_the_static_one_does_not() {
        let live: serde_json::Value = serde_json::from_str(&live_schema_text().unwrap()).unwrap();
        let theme = &live["$defs"]["ThemeSelectionContent"]["anyOf"];
        let names: Vec<&str> = theme[0]["enum"]
            .as_array()
            .expect("static theme names")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            names.contains(&oxikube_theme::registry::DEFAULT_DARK_THEME),
            "{names:?}"
        );
        let light = &theme[1]["properties"]["light"]["enum"];
        assert!(light.as_array().unwrap().contains(&serde_json::Value::Null));
        assert_eq!(
            live["properties"]["base_keymap"]["enum"],
            serde_json::json!(["default", "vim", null])
        );

        let fixed: serde_json::Value = serde_json::from_str(&schema_text().unwrap()).unwrap();
        assert!(fixed["$defs"]["ThemeSelectionContent"]["anyOf"][0]["enum"].is_null());
        assert_eq!(
            fixed["$defs"]["ThemeSelectionContent"]["anyOf"][0]["x-oxikube-dynamic-enum"],
            "themes"
        );
    }

    /// `settings.schema.json` is checked in and CI's `gen-settings-schema --check` fails when it
    /// is stale; this is the same comparison as a unit test, plus the per-cluster block (E06-S08).
    #[test]
    fn the_checked_in_schema_is_current_and_documents_the_cluster_block() {
        let generated = schema_text().unwrap();
        assert!(
            generated == oxikube_assets::settings_schema(),
            "settings.schema.json is stale: run `cargo xtask gen-settings-schema`"
        );
        let schema: serde_json::Value = serde_json::from_str(&generated).unwrap();
        let cluster = &schema["$defs"]["ClusterSettings"]["properties"];
        for key in [
            "display_name",
            "colour",
            "read_only",
            "default_namespace",
            "terminal_cwd",
            "node_shell_image",
            "node_shell_pull_secret",
            "node_shell",
            "prometheus",
            "accessible_namespaces",
            "exec_interactivity",
            "exec_in_read_only",
            "watch_budget",
        ] {
            assert!(
                cluster[key].is_object(),
                "`{key}` is not in the cluster block"
            );
        }
    }
}
