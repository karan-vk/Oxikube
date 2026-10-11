//! [`ConfigMapView`]: a ConfigMap's keys as the detail panel shows them (E12-S11).
//!
//! Built once per object version off the render path. Each text key carries what a row needs
//! (a one-line summary, the size, the line count) and what its preview needs (the value, the
//! language to colour it with, and the capped slice [`ConfigEntry::preview`]), so drawing a row
//! costs nothing and a 1 MiB value never reaches the renderer whole. `binaryData` is shown as a
//! size only.

use std::sync::Arc;

use super::{ViewError, check_kind, obj_of, sub};
use crate::resource::Resource;

/// The most bytes of a value a preview shows; the rest is cut at a character boundary and the
/// preview says so.
pub const PREVIEW_CAP: usize = 64 * 1024;

/// The most characters of a value's first line a row shows.
pub const SUMMARY_CHARS: usize = 96;

/// The most bytes of a value looked at to tell what language it is.
const SNIFF_BYTES: usize = 4096;

/// The language a value is previewed in: chosen from the key's extension, else from the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewLanguage {
    /// YAML (`.yaml`, `.yml`, or `key: value` lines).
    Yaml,
    /// JSON (`.json`, or a value that parses as an object or array).
    Json,
    /// Java properties, env files and INI (`.properties`, `.env`, `.ini`, `.conf`, `.cfg`, or
    /// `key=value` lines).
    Properties,
    /// TOML (`.toml`).
    Toml,
    /// A shell script (`.sh`, `.bash`, or a `#!` line).
    Shell,
    /// XML or HTML (`.xml`, `.html`, or a leading `<`).
    Xml,
    /// Anything else.
    Plain,
}

impl PreviewLanguage {
    /// The language of `value` stored under `key`.
    pub fn detect(key: &str, value: &str) -> Self {
        Self::from_key(key).unwrap_or_else(|| Self::sniff(value))
    }

    /// The language the extension of `key` names.
    fn from_key(key: &str) -> Option<Self> {
        let lower = key.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map(|(_, ext)| ext);
        match ext? {
            "yaml" | "yml" => Some(Self::Yaml),
            "json" => Some(Self::Json),
            "properties" | "env" | "ini" | "conf" | "cfg" => Some(Self::Properties),
            "toml" => Some(Self::Toml),
            "sh" | "bash" => Some(Self::Shell),
            "xml" | "html" | "xhtml" => Some(Self::Xml),
            _ => None,
        }
    }

    /// The language the content suggests.
    fn sniff(value: &str) -> Self {
        let head = head(value, SNIFF_BYTES);
        let trimmed = head.trim_start();
        if trimmed.starts_with("#!") {
            return Self::Shell;
        }
        if trimmed.starts_with('<') {
            return Self::Xml;
        }
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && value.len() <= PREVIEW_CAP
            && serde_json::from_str::<serde_json::Value>(value).is_ok()
        {
            return Self::Json;
        }
        let (mut colon, mut equals, mut lines) = (0usize, 0usize, 0usize);
        for line in head
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            lines += 1;
            if line.starts_with("- ") || line.contains(": ") || line.ends_with(':') {
                colon += 1;
            } else if let Some((name, _)) = line.split_once('=')
                && !name.is_empty()
                && !name.contains(char::is_whitespace)
            {
                equals += 1;
            }
        }
        if lines >= 2 && colon * 2 > lines {
            Self::Yaml
        } else if lines >= 1 && equals * 2 > lines {
            Self::Properties
        } else {
            Self::Plain
        }
    }

    /// The tree-sitter grammar that colours it, `None` for plain text. Only languages with a
    /// grammar in the build are named.
    pub fn grammar(self) -> Option<&'static str> {
        match self {
            Self::Yaml => Some("yaml"),
            Self::Json => Some("json"),
            _ => None,
        }
    }

    /// A short label for the badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Yaml => "YAML",
            Self::Json => "JSON",
            Self::Properties => "properties",
            Self::Toml => "TOML",
            Self::Shell => "shell",
            Self::Xml => "XML",
            Self::Plain => "text",
        }
    }
}

/// `text` cut to at most `max` bytes at a character boundary.
fn head(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// One text key of a ConfigMap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    /// The key (`app.yaml`).
    pub key: Arc<str>,
    /// The value.
    pub value: Arc<str>,
    /// The value's size in bytes.
    pub size: usize,
    /// How many lines the value has (an empty value has none).
    pub lines: usize,
    /// The language its preview is coloured in.
    pub language: PreviewLanguage,
    /// The first non-empty line, cut to [`SUMMARY_CHARS`] characters: what a row shows.
    pub summary: String,
}

impl ConfigEntry {
    fn new(key: &str, value: &str) -> Self {
        let first = value
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let mut summary: String = first.chars().take(SUMMARY_CHARS).collect();
        if first.chars().count() > SUMMARY_CHARS {
            summary.push('…');
        }
        Self {
            key: key.into(),
            size: value.len(),
            lines: value.lines().count(),
            language: PreviewLanguage::detect(key, value),
            summary,
            value: value.into(),
        }
    }

    /// The value capped at [`PREVIEW_CAP`] bytes: what the expanded preview shows.
    pub fn preview(&self) -> &str {
        head(&self.value, PREVIEW_CAP)
    }

    /// Whether [`preview`](Self::preview) leaves part of the value out.
    pub fn truncated(&self) -> bool {
        self.size > PREVIEW_CAP
    }
}

/// One `binaryData` key: shown as its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryEntry {
    /// The key.
    pub key: Arc<str>,
    /// The decoded size in bytes.
    pub size: usize,
}

/// A ConfigMap's keys, sorted by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigMapView {
    /// `immutable: true`.
    pub immutable: bool,
    /// The `data` keys.
    pub entries: Vec<ConfigEntry>,
    /// The `binaryData` keys.
    pub binary: Vec<BinaryEntry>,
}

impl ConfigMapView {
    /// Builds the view of a ConfigMap.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a core `ConfigMap`.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "ConfigMap", &[("", "ConfigMap")])?;
        let root = res.json();
        let mut entries: Vec<ConfigEntry> = obj_of(root, "data")
            .and_then(|data| data.as_object())
            .map(|data| {
                data.iter()
                    .filter_map(|(key, value)| Some(ConfigEntry::new(key, value.as_str()?)))
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let mut binary: Vec<BinaryEntry> = obj_of(root, "binaryData")
            .and_then(|data| data.as_object())
            .map(|data| {
                data.iter()
                    .filter_map(|(key, value)| {
                        Some(BinaryEntry {
                            key: key.into(),
                            size: base64_len(value.as_str()?),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        binary.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(Self {
            immutable: sub(root, "immutable").as_bool().unwrap_or(false),
            entries,
            binary,
        })
    }

    /// How many keys the ConfigMap holds, text and binary.
    pub fn len(&self) -> usize {
        self.entries.len() + self.binary.len()
    }

    /// Whether the ConfigMap holds no keys.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The decoded size of base64 `text`, from its length alone.
fn base64_len(text: &str) -> usize {
    let digits = text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .count();
    digits * 3 / 4
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn config(data: serde_json::Value) -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "c", "namespace": "ns"},
            "data": data
        }))
        .unwrap()
    }

    #[test]
    fn the_language_follows_the_key_then_the_content() {
        use PreviewLanguage::*;
        assert_eq!(PreviewLanguage::detect("app.yaml", "x"), Yaml);
        assert_eq!(PreviewLanguage::detect("a.JSON", "x"), Json);
        assert_eq!(PreviewLanguage::detect("db.properties", "x"), Properties);
        assert_eq!(PreviewLanguage::detect("run.sh", "x"), Shell);
        assert_eq!(PreviewLanguage::detect("note", "{\"a\": 1}"), Json);
        assert_eq!(PreviewLanguage::detect("note", "{not json"), Plain);
        assert_eq!(PreviewLanguage::detect("note", "a: 1\nb:\n  - x\n"), Yaml);
        assert_eq!(PreviewLanguage::detect("note", "A=1\nB=two\n"), Properties);
        assert_eq!(PreviewLanguage::detect("note", "#!/bin/sh\necho"), Shell);
        assert_eq!(PreviewLanguage::detect("note", "<xml/>"), Xml);
        assert_eq!(PreviewLanguage::detect("note", "just words"), Plain);
        assert_eq!(PreviewLanguage::detect("note", ""), Plain);
    }

    #[test]
    fn entries_are_sorted_and_summarised() {
        let long = "x".repeat(200);
        let view = ConfigMapView::from_resource(&config(json!({
            "b.txt": "\n  second line first\nmore\n",
            "a.yaml": "k: v\n",
            "c": long
        })))
        .unwrap();
        let keys: Vec<&str> = view.entries.iter().map(|e| &*e.key).collect();
        assert_eq!(keys, ["a.yaml", "b.txt", "c"]);
        assert_eq!(view.entries[1].summary, "second line first");
        assert_eq!(view.entries[1].lines, 3);
        assert_eq!(view.entries[2].summary.chars().count(), SUMMARY_CHARS + 1);
        assert!(view.entries[2].summary.ends_with('…'));
    }

    #[test]
    fn a_big_value_previews_capped_at_a_char_boundary() {
        let value = "é".repeat(PREVIEW_CAP);
        let view = ConfigMapView::from_resource(&config(json!({"big": value}))).unwrap();
        let entry = &view.entries[0];
        assert!(entry.truncated());
        assert!(entry.preview().len() <= PREVIEW_CAP);
        assert!(entry.preview().chars().all(|c| c == 'é'));
        assert_eq!(entry.size, PREVIEW_CAP * 2);
    }

    #[test]
    fn binary_data_is_a_size_only() {
        let resource = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "c"},
            "binaryData": {"blob": "//4=", "z": "AAAA"}, "immutable": true
        }))
        .unwrap();
        let view = ConfigMapView::from_resource(&resource).unwrap();
        assert!(view.immutable);
        assert_eq!(view.binary[0].size, 2);
        assert_eq!(view.binary[1].size, 3);
        assert!(view.entries.is_empty());
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn another_kind_is_refused() {
        let secret = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret", "metadata": {"name": "s"}
        }))
        .unwrap();
        assert!(ConfigMapView::from_resource(&secret).is_err());
    }
}
