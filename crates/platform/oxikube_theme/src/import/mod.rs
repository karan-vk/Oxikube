//! Importer for Zed theme-family JSON (schema v0.2.0).
//!
//! A file is `{ name, author, themes: [{ name, appearance, style: { .. }, oxikube?: { .. } }] }`.
//! Only the file *format* is shared with Zed (its `theme` crate is GPL and entangled); the
//! mapping onto [`ThemeTokens`] is the table in `table`. Parsing uses the lenient JSON parser
//! (comments, trailing commas), keys the file does not set fall back to One Dark / One Light,
//! and nothing a theme author wrote can make the import fail beyond the file not being a theme
//! family: bad values are collected in the [`ImportReport`].
//!
//! - `table`: the key -> token table (extend here).
//! - `theme`: one theme entry -> tokens (fallbacks, derived colours, the `oxikube` block).
//! - `syntax`: `players`, `accents` and `syntax` entries.
//! - `report`: [`ImportReport`], [`ImportDiagnostic`], [`ImportError`].

mod report;
mod syntax;
mod table;
mod theme;

use crate::appearance::Appearance;
use crate::tokens::ThemeTokens;
use serde::Deserialize as _;
use serde_json::Value;

pub use report::{ImportDiagnostic, ImportError, ImportReport};

/// The schema this importer implements.
pub const SCHEMA_VERSION: &str = "v0.2.0";

/// A parsed theme-family file.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFamily {
    /// Family name, e.g. `Ayu`.
    pub name: String,
    /// Author credit, empty when absent.
    pub author: String,
    /// The themes of the family, in file order.
    pub themes: Vec<ThemeTokens>,
}

/// The result of an import: the family and what was wrong with it.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedFamily {
    /// The themes that imported.
    pub family: ThemeFamily,
    /// Problems and unmapped keys found on the way.
    pub report: ImportReport,
}

/// Imports a Zed theme-family file; missing keys fall back to One Dark / One Light.
pub fn import_family(text: &str) -> Result<ImportedFamily, ImportError> {
    import_family_with(text, &|appearance| {
        ThemeTokens::fallback(appearance).clone()
    })
}

/// [`import_family`] with an explicit base per appearance (the fallback bootstrap and tests).
pub(crate) fn import_family_with(
    text: &str,
    base_for: &dyn Fn(Appearance) -> ThemeTokens,
) -> Result<ImportedFamily, ImportError> {
    let root = parse_lenient(text)?;
    let object = root.as_object().ok_or(ImportError::NotAThemeFamily)?;
    let entries = object
        .get("themes")
        .and_then(Value::as_array)
        .ok_or(ImportError::NotAThemeFamily)?;

    let mut report = ImportReport::default();
    if let Some(schema) = object.get("$schema").and_then(Value::as_str)
        && !schema.ends_with(&format!("/{SCHEMA_VERSION}.json"))
    {
        report.diagnostics.push(ImportDiagnostic::SchemaVersion {
            found: schema.to_owned(),
        });
    }

    let mut themes = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        match read_entry(entry) {
            Ok((name, appearance, style, oxikube)) => themes.push(theme::build_theme(
                name,
                appearance,
                style,
                oxikube,
                base_for(appearance),
                &mut report,
            )),
            Err(reason) => report
                .diagnostics
                .push(ImportDiagnostic::SkippedTheme { index, reason }),
        }
    }
    Ok(ImportedFamily {
        family: ThemeFamily {
            name: string_field(object, "name"),
            author: string_field(object, "author"),
            themes,
        },
        report,
    })
}

/// Imports the bundled theme called `name` over `base` (no fallback involved). `None` when no
/// bundled file has it.
pub(crate) fn import_bundled_over(name: &str, base: ThemeTokens) -> Option<ThemeTokens> {
    let appearance = base.appearance;
    oxikube_assets::BUNDLED_THEME_FAMILIES
        .iter()
        .find_map(|file| {
            import_family_with(file.json, &|_| ThemeTokens::placeholder(appearance))
                .ok()?
                .family
                .themes
                .into_iter()
                .find(|theme| theme.name == name)
        })
}

type Entry<'a> = (
    &'a str,
    Appearance,
    &'a serde_json::Map<String, Value>,
    Option<&'a serde_json::Map<String, Value>>,
);

/// Name, appearance, `style` and `oxikube` of one `themes[]` entry.
fn read_entry(entry: &Value) -> Result<Entry<'_>, String> {
    let object = entry.as_object().ok_or("not an object")?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or("missing `name`")?;
    let appearance = match object.get("appearance").and_then(Value::as_str) {
        Some("dark") => Appearance::Dark,
        Some("light") => Appearance::Light,
        other => {
            return Err(format!(
                "`appearance` must be \"light\" or \"dark\", got {other:?}"
            ));
        }
    };
    static EMPTY: std::sync::LazyLock<serde_json::Map<String, Value>> =
        std::sync::LazyLock::new(serde_json::Map::new);
    let style = match object.get("style") {
        None | Some(Value::Null) => &*EMPTY,
        Some(Value::Object(style)) => style,
        Some(_) => return Err("`style` must be an object".to_owned()),
    };
    let oxikube = object.get("oxikube").and_then(Value::as_object);
    Ok((name, appearance, style, oxikube))
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Parses JSON with comments and trailing commas (the parser Zed uses for theme files).
fn parse_lenient(text: &str) -> Result<Value, ImportError> {
    let mut deserializer = serde_json_lenient::Deserializer::from_str(text);
    let value =
        Value::deserialize(&mut deserializer).map_err(|err| ImportError::Json(err.to_string()))?;
    deserializer
        .end()
        .map_err(|err| ImportError::Json(err.to_string()))?;
    Ok(value)
}

#[cfg(test)]
mod tests;
