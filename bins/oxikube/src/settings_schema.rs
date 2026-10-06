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

use oxikube_settings::{RegisteredSetting, SettingsStore, schema::to_schema_text};

use crate::cli::Print;

/// Runs a [`Print`] request and returns the exit code.
pub fn print(what: Print) -> ExitCode {
    let text = match what {
        Print::SettingsSchema => schema_text().map_err(|err| err.to_string()),
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
}
