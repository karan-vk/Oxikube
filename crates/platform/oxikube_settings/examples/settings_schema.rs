//! Print the JSON schema of `settings.json` for every registered setting.
//!
//! Run by `cargo xtask gen-settings-schema`, which writes the output to
//! `oxikube_assets::SETTINGS_SCHEMA_PATH` (or, with `--check`, compares it).
#![allow(clippy::print_stdout)]

use oxikube_settings::SettingsStore;
use oxikube_settings::schema::to_schema_text;

fn main() -> Result<(), oxikube_domain::OxiError> {
    let store = SettingsStore::new(oxikube_assets::default_settings())?;
    print!("{}", to_schema_text(&store.json_schema()));
    Ok(())
}
