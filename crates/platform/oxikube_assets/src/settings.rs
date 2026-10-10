//! Settings assets (E05-S06), embedded with `include_str!` so reading them costs nothing at
//! startup.

/// Path of the generated settings schema, relative to the workspace root. `cargo xtask
/// gen-settings-schema` writes it; [`settings_schema`] embeds it.
pub const SETTINGS_SCHEMA_PATH: &str =
    "crates/platform/oxikube_assets/assets/settings/settings.schema.json";

/// The embedded default settings (`assets/settings/default.json`, JSON with comments).
pub fn default_settings() -> &'static str {
    include_str!("../assets/settings/default.json")
}

/// The commented template for a user `settings.json` created on first run.
pub fn initial_user_settings_content() -> &'static str {
    include_str!("../assets/settings/initial_user_settings.json")
}

/// The generated JSON schema for `settings.json` (`assets/settings/settings.schema.json`).
pub fn settings_schema() -> &'static str {
    include_str!("../assets/settings/settings.schema.json")
}

/// The JSON schema for the user's `aliases.json` (`assets/settings/aliases.schema.json`, E11-S04),
/// for editors; `oxikube_settings::aliases` is the validator the app runs.
pub fn aliases_schema() -> &'static str {
    include_str!("../assets/settings/aliases.schema.json")
}
