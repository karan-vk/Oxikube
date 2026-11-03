//! [`SettingsPageModel`]: the settings page as plain data, derived from the settings schema.
//!
//! Every registered setting reaches the schema (`SettingsStore::json_schema`, with the field
//! doc comments as descriptions), so the page needs no registration of its own: one section per
//! top-level key whose value is an object (`terminal`, `editor`, `helm`, ...), and a "General"
//! section, first, for the top-level fields that are not (`theme`, `ui_scale`, `read_only`, ...).
//! `clusters` and `$schema` are not settings and are left out. Fields are one level deep; a
//! nested object is one field of kind [`FieldKind::Map`] that the page edits as JSON until a
//! control exists for it (E21-S05).

use gpui::SharedString;
use serde_json::Value;

use super::schema_walk::Definitions;
use super::titles::{GENERAL_KEY, humanize, section_meta};

/// Top-level keys that are not settings.
const HIDDEN_KEYS: &[&str] = &["$schema", "clusters"];

/// What kind of value a field holds, from its schema: what control it gets (E21-S05).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// `true` or `false`.
    Toggle,
    /// A whole number.
    Integer,
    /// Any number.
    Number,
    /// Free text (a name, a path, a colour).
    Text,
    /// One of a fixed set of strings.
    Choice(Vec<SharedString>),
    /// A list of values.
    List,
    /// An object: a map, or a struct nested one level further.
    Map,
    /// A shape with no control of its own (a union of several types).
    Other,
}

impl FieldKind {
    /// A short name of the kind, shown next to a field whose control has not landed yet.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Toggle => "on / off",
            Self::Integer => "whole number",
            Self::Number => "number",
            Self::Text => "text",
            Self::Choice(_) => "choice",
            Self::List => "list",
            Self::Map => "object",
            Self::Other => "JSON",
        }
    }

    /// Whether the field is edited in `settings.json` rather than on the page: the shapes no
    /// control will cover (the "Edit in JSON" fallback).
    pub fn edited_in_json(&self) -> bool {
        matches!(self, Self::Map | Self::Other)
    }
}

/// One setting on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageField {
    /// The dotted path in `settings.json` (`terminal.font_size`, `ui_scale`).
    pub path: SharedString,
    /// Its title ("Font size").
    pub title: SharedString,
    /// Its description, from the setting's doc comment; empty when it has none.
    pub description: SharedString,
    /// What it holds.
    pub kind: FieldKind,
}

/// One section of the page: a top-level key of `settings.json`, or "General".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSection {
    /// The top-level key (`terminal`), or `general`.
    pub key: SharedString,
    /// Its title in the navigation ("Terminal").
    pub title: SharedString,
    /// One line about it, when there is one.
    pub description: Option<SharedString>,
    /// Its fields, in key order.
    pub fields: Vec<PageField>,
}

/// The whole page: its sections in navigation order. See the [page module](crate::page).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsPageModel {
    sections: Vec<PageSection>,
}

impl SettingsPageModel {
    /// The page for `schema`, the document `SettingsStore::json_schema` returns. A schema without
    /// `properties` gives an empty page.
    pub fn from_schema(schema: &Value) -> Self {
        let definitions = Definitions::of(schema);
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return Self::default();
        };
        let mut general = Vec::new();
        let mut sections = Vec::new();
        for (key, node) in properties {
            if HIDDEN_KEYS.contains(&key.as_str()) {
                continue;
            }
            let resolved = definitions.resolve(node);
            match resolved.get("properties").and_then(Value::as_object) {
                Some(fields) => {
                    let mut fields: Vec<PageField> = fields
                        .iter()
                        .map(|(name, field)| {
                            page_field(&format!("{key}.{name}"), name, field, &definitions)
                        })
                        .collect();
                    // Key order whatever order the schema lists them in (the store's is sorted).
                    fields.sort_by(|a, b| a.path.cmp(&b.path));
                    let description = definitions.description_of(node);
                    sections.push(section(key, description, fields));
                }
                None => general.push(page_field(key, key, node, &definitions)),
            }
        }
        sections.sort_by(|a, b| a.title.cmp(&b.title));
        general.sort_by(|a, b| a.path.cmp(&b.path));
        if !general.is_empty() {
            let general = PageSection {
                key: GENERAL_KEY.into(),
                title: "General".into(),
                description: Some(
                    "Appearance, keymap and the defaults every cluster starts from.".into(),
                ),
                fields: general,
            };
            sections.insert(0, general);
        }
        Self { sections }
    }

    /// A page of `sections` as given (tests build large pages this way).
    #[cfg(test)]
    pub(crate) fn from_sections(sections: Vec<PageSection>) -> Self {
        Self { sections }
    }

    /// The sections, in navigation order.
    pub fn sections(&self) -> &[PageSection] {
        &self.sections
    }

    /// The section at `ix`.
    pub fn section(&self, ix: usize) -> Option<&PageSection> {
        self.sections.get(ix)
    }

    /// The field at `field` of the section at `section`.
    pub fn field(&self, section: usize, field: usize) -> Option<&PageField> {
        self.sections.get(section)?.fields.get(field)
    }

    /// The index of the section whose key is `key`.
    pub fn section_index(&self, key: &str) -> Option<usize> {
        self.sections.iter().position(|section| section.key == key)
    }

    /// The section and field indices of the field at the dotted `path`.
    pub fn find_field(&self, path: &str) -> Option<(usize, usize)> {
        self.sections.iter().enumerate().find_map(|(section, s)| {
            let field = s.fields.iter().position(|field| field.path == path)?;
            Some((section, field))
        })
    }

    /// How many fields the page has in all.
    pub fn field_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.fields.len())
            .sum()
    }
}

/// The section `key` with `fields`, titled and described by the page's metadata, else from the
/// key and the schema.
fn section(key: &str, description: Option<&str>, fields: Vec<PageField>) -> PageSection {
    let (title, description) = match section_meta(key) {
        Some((title, blurb)) => (title.to_owned(), Some(blurb.to_owned())),
        None => (
            humanize(key),
            description
                .filter(|text| !text.starts_with("What one settings layer says"))
                .map(one_line),
        ),
    };
    PageSection {
        key: SharedString::from(key.to_owned()),
        title: title.into(),
        description: description.map(SharedString::from),
        fields,
    }
}

fn page_field(path: &str, name: &str, node: &Value, definitions: &Definitions<'_>) -> PageField {
    PageField {
        path: SharedString::from(path.to_owned()),
        title: humanize(name).into(),
        description: definitions
            .description_of(node)
            .map(one_line)
            .unwrap_or_default()
            .into(),
        kind: definitions.kind_of(node),
    }
}

/// A doc comment's text as one line: its hard line breaks become spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
