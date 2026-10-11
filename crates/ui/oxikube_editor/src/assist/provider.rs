//! [`ManifestAssist`]: hover and completion for a manifest editor, over the schemas it knows.
//!
//! Implements the wrapper's [`EditorAssist`], so the library's hover and completion providers
//! call it on the background executor. The schemas are those the editor has already fetched
//! (`SchemaPort`, on the Kubernetes runtime); nothing here fetches, and nothing here touches the
//! UI thread. The last parse is kept, keyed by the text, so hover after a validation pass (which
//! hands its parse over) and repeated hovers over an unchanged buffer parse nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_ui::editor::{CompletionEntry, EditorAssist, EntryKind, HoverInfo, TextSnapshot};

use super::buffer::Buffer;
use super::complete::{CompletionKind, can_complete, complete_at};
use super::hover::hover_at;
use crate::yaml::{ParseResult, parse_shared};

/// The schemas by kind: `Some` when the cluster served one, `None` when it has none.
type Schemas = HashMap<Gvk, Option<Arc<JsonSchema>>>;

/// Hover and completion over the schemas an editor has fetched. Share it (`Arc`) between the
/// editor view, which feeds it, and the code editor, which asks it.
#[derive(Default)]
pub struct ManifestAssist {
    schemas: RwLock<Schemas>,
    last: Mutex<Option<(TextSnapshot, Arc<ParseResult>)>>,
}

impl ManifestAssist {
    /// An assist that knows no schema yet.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Replaces the known schemas (the editor calls it when one arrives).
    pub fn set_schemas(&self, known: &Schemas) {
        if let Ok(mut schemas) = self.schemas.write() {
            schemas.clone_from(known);
        }
    }

    /// Keeps the model of `text`, which a validation pass just built, for the next hover.
    pub fn remember(&self, text: TextSnapshot, parsed: Arc<ParseResult>) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some((text, parsed));
        }
    }

    fn cached(&self, text: &TextSnapshot) -> Option<Arc<ParseResult>> {
        let last = self.last.lock().ok()?;
        let (kept, parsed) = last.as_ref()?;
        kept.same_text(text).then(|| Arc::clone(parsed))
    }

    fn schema_for(&self, gvk: &Gvk) -> Option<Arc<JsonSchema>> {
        self.schemas.read().ok()?.get(gvk)?.clone()
    }
}

impl EditorAssist for ManifestAssist {
    fn hover(&self, text: &TextSnapshot, offset: usize) -> Option<HoverInfo> {
        let parsed = self.cached(text).unwrap_or_else(|| {
            let parsed = Arc::new(parse_shared(text.to_text()));
            self.remember(text.clone(), Arc::clone(&parsed));
            parsed
        });
        let doc = hover_at(&parsed, |gvk| self.schema_for(gvk), offset)?;
        Some(HoverInfo {
            range: doc.range.clone(),
            markdown: doc.to_markdown(),
        })
    }

    fn complete(&self, text: &TextSnapshot, offset: usize) -> Vec<CompletionEntry> {
        let source = text.to_text();
        if !can_complete(&source, offset) {
            return Vec::new();
        }
        let buffer = match self.cached(text) {
            Some(parsed) => Buffer::parsed(parsed),
            None => Buffer::new(source),
        };
        let items = complete_at(&buffer, |gvk| self.schema_for(gvk), offset);
        if let Some(parsed) = buffer.parse_if_done() {
            self.remember(text.clone(), Arc::clone(parsed));
        }
        items
            .into_iter()
            .map(|item| CompletionEntry {
                label: item.label,
                kind: match item.kind {
                    CompletionKind::Field => EntryKind::Field,
                    CompletionKind::Value => EntryKind::Value,
                    CompletionKind::Snippet => EntryKind::Snippet,
                },
                detail: item.detail,
                documentation: item.documentation,
                insert_text: item.insert_text,
                replace: item.replace,
            })
            .collect()
    }
}
