//! The user's `aliases.json` (E11-S04): your own names for the `:` jump bar.
//!
//! ```jsonc
//! {
//!   // name -> a resource (group/version/plural; `v1/pods` for the core group) ...
//!   "prodpods": "v1/pods",
//!   "dep":      { "gvr": "apps/v1/deployments" },
//!   // ... or a jump-bar command line, k9s style (`fred: pod fred app=blee`)
//!   "fred":     "pod fred app=blee",
//!   "blee":     { "command": "pod", "args": ["fred", "app=blee"] }
//! }
//! ```
//!
//! It sits next to `settings.json` and `keymap.json`, is JSON with comments, and is reloaded when
//! it is saved. A bad entry is skipped and reported with its line; the others still load, and a
//! file that is not valid JSON keeps the last good aliases. This crate only parses and watches:
//! the app layer cannot depend on it, so the binary hands the parsed aliases to
//! `oxikube_app::search::aliases::AliasRegistry::set_user_aliases`.
//!
//! | Module | Holds |
//! |---|---|
//! | `parse` | [`parse_aliases`]: text to [`UserAlias`]es and [`AliasDiagnostic`]s |
//! | `lines` | the line of each top-level key of a JSON-with-comments object |
//! | `file` | [`UserAliasesFile`]: read, hot reload (a `watch: false` constructor for tests), explicit reload |
//!
//! The format is described for editors by `oxikube_assets::aliases_schema()`.

mod file;
mod lines;
mod parse;

pub use file::{LoadedAliases, UserAliasesFile};
pub use parse::{
    AliasDiagnostic, MAX_ALIAS_NAME_LEN, ParsedAliases, UserAlias, alias_name_problem,
    parse_aliases,
};

/// File name of the user aliases inside the config dir.
pub const ALIASES_FILE_NAME: &str = "aliases.json";

/// The user aliases inside `config_dir`.
pub fn user_aliases_path(config_dir: &std::path::Path) -> std::path::PathBuf {
    config_dir.join(ALIASES_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema editors use and the validator the app runs describe the same format: its own
    /// example parses cleanly, and the fields it names are the ones the validator reads.
    #[test]
    fn the_shipped_schema_describes_what_the_parser_accepts() {
        let schema: serde_json::Value =
            serde_json::from_str(oxikube_assets::aliases_schema()).expect("schema is JSON");
        let example = schema["examples"][0].to_string();
        let parsed = parse_aliases(&example).expect("the example parses");
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        assert_eq!(parsed.aliases.len(), 4);

        let forms = schema["additionalProperties"]["oneOf"].as_array().unwrap();
        let fields: Vec<_> = forms
            .iter()
            .flat_map(|form| form["properties"].as_object().into_iter().flatten())
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(fields, ["gvr", "command", "args"]);
    }

    #[test]
    fn the_name_pattern_in_the_schema_matches_the_validator() {
        let schema: serde_json::Value =
            serde_json::from_str(oxikube_assets::aliases_schema()).unwrap();
        let pattern = schema["propertyNames"]["anyOf"][1]["pattern"]
            .as_str()
            .unwrap();
        assert_eq!(pattern, r"^[^\s/@=,]+$");
        assert_eq!(
            schema["propertyNames"]["anyOf"][1]["maxLength"],
            MAX_ALIAS_NAME_LEN
        );
    }

    #[test]
    fn the_file_lives_next_to_settings_json() {
        let path = user_aliases_path(std::path::Path::new("/c"));
        assert_eq!(path, std::path::Path::new("/c/aliases.json"));
    }
}
