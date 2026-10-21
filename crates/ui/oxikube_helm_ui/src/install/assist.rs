//! [`ValuesAssist`]: hover docs and completion for the values editor, from the chart's
//! [`ValuesDocs`].
//!
//! Hover parses the buffer with the manifest editor's spanned YAML model
//! ([`oxikube_editor::yaml`]) and documents the key or value under the pointer. Completion reads
//! the cursor's line lexically ([`site_at`]): typing a key offers the keys the chart documents
//! under the enclosing map; typing a value offers the key's allowed values (`enum`) or `true` /
//! `false`. Both run on the background executor (the editor's assist bridge); the last parse is
//! kept for the text it was made from, so repeated hovers over an unchanged buffer parse nothing.

use std::ops::Range;
use std::sync::{Arc, Mutex, RwLock};

use oxikube_domain::helm::ValuesDocs;
use oxikube_editor::yaml::{JsonPath, ParseResult, PathSegment, parse_shared};
use oxikube_ui::editor::{CompletionEntry, EditorAssist, EntryKind, HoverInfo, TextSnapshot};

/// Hover and completion over the chart's values documentation. Share it (`Arc`) between the tab,
/// which sets the docs once they are read, and the code editor, which asks it.
#[derive(Default)]
pub struct ValuesAssist {
    docs: RwLock<Arc<ValuesDocs>>,
    last: Mutex<Option<(TextSnapshot, Arc<ParseResult>)>>,
}

impl ValuesAssist {
    /// An assist that documents nothing yet.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Replaces the documentation (the chart's defaults were read and parsed).
    pub fn set_docs(&self, docs: Arc<ValuesDocs>) {
        if let Ok(mut current) = self.docs.write() {
            *current = docs;
        }
    }

    /// The documentation in use.
    pub fn docs(&self) -> Arc<ValuesDocs> {
        self.docs.read().map(|d| d.clone()).unwrap_or_default()
    }

    fn parsed(&self, text: &TextSnapshot) -> Arc<ParseResult> {
        if let Ok(last) = self.last.lock()
            && let Some((kept, parsed)) = last.as_ref()
            && kept.same_text(text)
        {
            return parsed.clone();
        }
        let parsed = Arc::new(parse_shared(text.to_text()));
        if let Ok(mut last) = self.last.lock() {
            *last = Some((text.clone(), parsed.clone()));
        }
        parsed
    }
}

/// `image.tag` for a path of keys; `None` when the path goes through a list item (the chart's
/// docs describe the values' own maps, not list elements).
pub fn dotted(path: &JsonPath) -> Option<String> {
    let mut keys = Vec::with_capacity(path.0.len());
    for segment in &path.0 {
        match segment {
            PathSegment::Key(key) => keys.push(&**key),
            PathSegment::Index(_) => return None,
        }
    }
    (!keys.is_empty()).then(|| keys.join("."))
}

/// What the cursor is typing, read off its line and the lines above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    /// A key under the map at `parent` (`""` for the top level); `word` is the bytes typed so far.
    Key {
        /// The dotted path of the enclosing map.
        parent: String,
        /// The bytes of the partial key.
        word: Range<usize>,
    },
    /// The value of the key at `path`; `word` is the bytes typed so far.
    Value {
        /// The dotted path of the key.
        path: String,
        /// The bytes of the partial value.
        word: Range<usize>,
    },
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_word(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The completion site at `offset` of `text`, `None` inside a comment, a list item or anything
/// that is not a key or a value being typed.
pub fn site_at(text: &str, offset: usize) -> Option<Site> {
    let offset = offset.min(text.len());
    if !text.is_char_boundary(offset) {
        return None;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |at| at + 1);
    let before = &text[line_start..offset];
    let indent = indent_of(before);
    let typed = &before[indent..];
    if typed.starts_with('#') || typed.starts_with('-') {
        return None;
    }
    let parent = parent_path(&text[..line_start], indent)?;
    if let Some((key, value)) = typed.split_once(':') {
        let value = value.trim_start();
        if !is_word(key) || !is_word(value) {
            return None;
        }
        let path = if parent.is_empty() {
            key.to_owned()
        } else {
            format!("{parent}.{key}")
        };
        return Some(Site::Value {
            path,
            word: offset - value.len()..offset,
        });
    }
    is_word(typed).then(|| Site::Key {
        parent,
        word: offset - typed.len()..offset,
    })
}

/// The dotted path of the map a line indented by `indent` sits in, read from the lines `above`
/// (the nearest less-indented `key:` line, recursively). `None` when the line is inside a list.
fn parent_path(above: &str, indent: usize) -> Option<String> {
    let mut keys = Vec::new();
    let mut level = indent;
    for line in above.lines().rev() {
        let body = line.split(" #").next().unwrap_or_default().trim();
        if level == 0 {
            break;
        }
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let line_indent = indent_of(line);
        if line_indent >= level {
            continue;
        }
        if body.starts_with('-') {
            return None;
        }
        let key = body.strip_suffix(':')?;
        keys.push(key.trim_matches(|c| c == '"' || c == '\'').to_owned());
        level = line_indent;
    }
    keys.reverse();
    Some(keys.join("."))
}

impl EditorAssist for ValuesAssist {
    fn hover(&self, text: &TextSnapshot, offset: usize) -> Option<HoverInfo> {
        let docs = self.docs();
        if docs.is_empty() {
            return None;
        }
        let parsed = self.parsed(text);
        let at = parsed.offset_to_path(offset)?;
        let path = dotted(&at.path)?;
        let markdown = docs.hover_markdown(&path)?;
        let range = parsed
            .key_at(offset)
            .map(|key| key.key_span)
            .or_else(|| parsed.path_to_span(&at))
            .unwrap_or(offset..offset);
        Some(HoverInfo { range, markdown })
    }

    fn complete(&self, text: &TextSnapshot, offset: usize) -> Vec<CompletionEntry> {
        let docs = self.docs();
        let text = text.to_text();
        match site_at(&text, offset) {
            Some(Site::Key { parent, word }) => {
                let typed = text[word.clone()].to_lowercase();
                docs.children(&parent)
                    .filter(|(key, _)| key.to_lowercase().starts_with(&typed))
                    .map(|(key, doc)| CompletionEntry {
                        label: key.to_owned(),
                        kind: EntryKind::Field,
                        detail: doc.type_hint.clone(),
                        documentation: (!doc.description.is_empty())
                            .then(|| doc.description.clone()),
                        insert_text: format!("{key}: "),
                        replace: word.clone(),
                    })
                    .collect()
            }
            Some(Site::Value { path, word }) => {
                let Some(doc) = docs.get(&path) else {
                    return Vec::new();
                };
                let choices: Vec<String> = if !doc.allowed.is_empty() {
                    doc.allowed.clone()
                } else if matches!(doc.type_hint.as_deref(), Some("bool" | "boolean")) {
                    vec!["true".to_owned(), "false".to_owned()]
                } else {
                    Vec::new()
                };
                let typed = &text[word.clone()];
                choices
                    .into_iter()
                    .filter(|choice| choice.starts_with(typed))
                    .map(|choice| CompletionEntry {
                        label: choice.clone(),
                        kind: EntryKind::Value,
                        detail: doc.type_hint.clone(),
                        documentation: None,
                        insert_text: choice,
                        replace: word.clone(),
                    })
                    .collect()
            }
            None => Vec::new(),
        }
    }
}
