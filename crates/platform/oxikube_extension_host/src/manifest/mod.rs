//! `extension.toml`: the schema, the parser and its validation.
//!
//! Pure parsing, no I/O: callers read the file and pass the text to
//! [`ExtensionManifest::parse`]. Every table rejects unknown fields, so a typo
//! is reported at install time with the field name and the line toml found it
//! on. Existence of the files named by `themes` / `icon_themes` is checked by
//! the installer, not here.
//!
//! # Capability patterns
//!
//! Capability patterns are globs where `*` matches any run of characters
//! (including none) and every other character matches itself. An empty
//! pattern, or a capability that lists no patterns, is rejected.

mod capability;
mod error;
mod validate;

pub use capability::ExtensionCapability;
pub use error::ManifestError;

use oxikube_domain::command::CommandScope;
use semver::Version;
use serde::Deserialize;

/// The only manifest schema version this host understands.
pub const SCHEMA_VERSION: u32 = 1;

/// A parsed, validated `extension.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionManifest {
    /// Unique id, `[a-z0-9-]+`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Extension version.
    pub version: Version,
    /// Manifest schema version; must be [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Authors, free text.
    #[serde(default)]
    pub authors: Vec<String>,
    /// Source repository URL.
    #[serde(default)]
    pub repository: Option<String>,
    /// The wasm library, absent for declarative-only extensions.
    #[serde(default)]
    pub lib: Option<LibSpec>,
    /// Theme JSON files, relative to the extension directory.
    #[serde(default)]
    pub themes: Vec<String>,
    /// Icon theme JSON files, relative to the extension directory.
    #[serde(default)]
    pub icon_themes: Vec<String>,
    /// Commands the extension contributes (`[[commands]]`).
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    /// MCP context servers the extension contributes (`[[context_servers]]`).
    #[serde(default)]
    pub context_servers: Vec<ContextServerDecl>,
    /// What the extension may do beyond pure computation.
    #[serde(default)]
    pub capabilities: Vec<ExtensionCapability>,
}

/// The `[lib]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibSpec {
    /// Source language of the component.
    pub kind: LibKind,
    /// `oxikube_extension_api` version the component was built against.
    pub version: Version,
}

/// Source language of an extension library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibKind {
    /// A Rust crate compiled to a wasm component.
    Rust,
}

/// One `[[commands]]` entry: a plain id (the host namespaces it later).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDecl {
    /// Plain id, `[a-z0-9-]+`; no `:`.
    pub id: String,
    /// Palette title.
    pub title: String,
    /// Palette description.
    #[serde(default)]
    pub description: String,
    /// Where the palette offers the command; defaults to global.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<CommandScope>,
}

fn default_scopes() -> Vec<CommandScope> {
    vec![CommandScope::Global]
}

/// One `[[context_servers]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextServerDecl {
    /// Plain id, `[a-z0-9-]+`.
    pub id: String,
}

impl ExtensionManifest {
    /// Parses and validates the text of an `extension.toml`.
    ///
    /// # Errors
    ///
    /// A [`ManifestError`] naming the offending field (and line, for toml
    /// errors) when the text is not valid toml, has an unknown or missing
    /// field, or breaks a validation rule.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        // A plain table parse separates syntax errors from schema errors.
        text.parse::<toml::Table>()
            .map_err(|e| ManifestError::Syntax(e.to_string()))?;
        let manifest: Self = toml::from_str(text).map_err(ManifestError::from_schema)?;
        validate::validate(&manifest)?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests;
