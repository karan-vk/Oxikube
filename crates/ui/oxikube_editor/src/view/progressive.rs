//! Progressive validation (E10-S11): a pass over one buffer version, split into chunks of
//! [`Unit`]s that run one after the other on the background executor, the objects on screen
//! first.
//!
//! - [`plan`] (background): the buffer's units in the order they are checked. A buffer under
//!   [`LARGE_BUFFER_BYTES`] is one chunk, shown when it is done, exactly as a one-shot validation
//!   would. A larger one (a `kubectl get -o yaml` dump) starts with the units overlapping the
//!   visible lines, then the rest in buffer order, about [`CHUNK_BYTES`] of objects per chunk.
//! - [`check_chunk`] (background): one chunk against the known schemas, cooperative with the
//!   pass's [`CancelToken`], so a stale chunk stops within a few hundred nodes.
//! - [`Progress`] (UI thread): the findings so far. After each chunk the view shows them (a large
//!   buffer fills in as it goes), until the pass is complete or the diagnostics cap is reached.
//!
//! Pure: no gpui, so the plan, the chunking and the accumulation are tested without a window.

use std::ops::Range;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_ui::editor::{DiagnosticLevel, EditorDiagnostic};

use super::validation::{KnownSchemas, Validation};
use crate::cancel::{CancelToken, Cancelled};
use crate::json::{self, Language, LanguageMode};
use crate::open::masked_diagnostics;
use crate::validate::{self, Severity, Unit, ValidateOptions};
use crate::yaml::ParseResult;

/// From this size on a buffer is validated in chunks, the visible objects first, and the toolbar
/// says the file is large. Below it a pass is one chunk.
pub const LARGE_BUFFER_BYTES: usize = 1024 * 1024;

/// About how many bytes of objects one chunk of a large buffer holds: a few milliseconds of
/// validation, so a cancelled pass gives its core back quickly and results arrive steadily.
pub(crate) const CHUNK_BYTES: usize = 256 * 1024;

/// How far a pass has got: `done` of `total` chunks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Chunks checked.
    pub done: usize,
    /// Chunks in the pass.
    pub total: usize,
}

impl Coverage {
    /// Every chunk was checked (or the diagnostics cap ended the pass).
    pub fn is_complete(&self) -> bool {
        self.done >= self.total
    }
}

/// What one pass checks: the parsed buffer at `version` and its units, chunked in check order.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    /// The buffer version parsed.
    pub version: u64,
    /// The parse (shared with the parse cache).
    pub parsed: Arc<ParseResult>,
    /// How the language was chosen, and what the buffer was read as.
    pub mode: LanguageMode,
    /// The language the buffer is read as (the mode's, or what the content looks like).
    pub language: Language,
    /// What is known from the parse alone, shown from the first chunk on: the syntax errors (for
    /// JSON the strict check's, in place of the YAML parser's) and, for a Secret opened masked,
    /// the placeholders still in it.
    pub base: Vec<EditorDiagnostic>,
    /// The units, in the order they are checked; never empty (a buffer without objects has one
    /// empty chunk, so its syntax errors are still shown at the end of the pass).
    pub chunks: Vec<Vec<Unit>>,
}

/// The plan for `parsed` (the buffer at `version`), read as the language `mode` gives and, when
/// `masking`, checked for masked placeholders; the units overlapping `visible` (byte range of the
/// lines on screen) first when the buffer is large. Runs on the background executor.
pub(crate) fn plan(
    version: u64,
    parsed: Arc<ParseResult>,
    visible: Option<Range<usize>>,
    mode: LanguageMode,
    masking: bool,
) -> Plan {
    let language = mode.resolve(parsed.text());
    let mut base: Vec<EditorDiagnostic> =
        validate::syntax_diagnostics(&parsed, &ValidateOptions::default())
            .iter()
            .map(to_editor)
            .collect();
    if language == Language::Json
        && let Some(error) = json::first_error(parsed.text())
    {
        base.retain(|d| d.code.as_deref() != Some("syntax"));
        base.push(
            EditorDiagnostic::new(error.span, DiagnosticLevel::Error, error.message)
                .with_code("syntax"),
        );
    }
    if masking {
        base.extend(masked_diagnostics(&parsed));
    }
    let all = validate::units(&parsed);
    let chunks = if parsed.text().len() < LARGE_BUFFER_BYTES {
        vec![all]
    } else {
        chunk(all, visible, CHUNK_BYTES)
    };
    Plan {
        version,
        parsed,
        mode,
        language,
        base,
        chunks,
    }
}

/// `units` in check order: those overlapping `visible` as the first chunk, then the others in
/// buffer order, a new chunk whenever the current one holds `bytes` of objects. Never empty.
pub(crate) fn chunk(
    units: Vec<Unit>,
    visible: Option<Range<usize>>,
    bytes: usize,
) -> Vec<Vec<Unit>> {
    let overlaps = |unit: &Unit| {
        visible
            .as_ref()
            .is_some_and(|v| unit.span.start <= v.end && v.start <= unit.span.end)
    };
    let (first, rest): (Vec<Unit>, Vec<Unit>) = units.into_iter().partition(overlaps);
    let mut chunks: Vec<Vec<Unit>> = Vec::new();
    if !first.is_empty() {
        chunks.push(first);
    }
    let mut current: Vec<Unit> = Vec::new();
    let mut size = 0;
    for unit in rest {
        size += unit.span.len();
        current.push(unit);
        if size >= bytes {
            chunks.push(std::mem::take(&mut current));
            size = 0;
        }
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// What one chunk found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Findings {
    /// Schema diagnostics of the chunk's units, in buffer order.
    pub diagnostics: Vec<EditorDiagnostic>,
    /// Kinds of the chunk whose schema is not known yet.
    pub missing: Vec<Gvk>,
}

/// Checks `chunk` of `parsed` against the `known` schemas, reporting at most `room` diagnostics.
/// Stops with [`Cancelled`] at the next poll once `cancel` is cancelled. Runs on the background
/// executor.
pub(crate) fn check_chunk(
    parsed: &ParseResult,
    chunk: &[Unit],
    known: &KnownSchemas,
    room: usize,
    cancel: &CancelToken,
) -> Result<Findings, Cancelled> {
    let mut missing: Vec<Gvk> = Vec::new();
    let opts = ValidateOptions {
        max_diagnostics: room,
        ..ValidateOptions::default()
    };
    let found = validate::validate_units(
        parsed,
        chunk,
        |gvk| match known.get(gvk) {
            Some(schema) => schema.clone(),
            None => {
                if !missing.contains(gvk) {
                    missing.push(gvk.clone());
                }
                None
            }
        },
        &opts,
        cancel,
    )?;
    Ok(Findings {
        diagnostics: found.iter().map(to_editor).collect(),
        missing,
    })
}

/// The editor's form of a validator finding.
pub(crate) fn to_editor(found: &validate::Diagnostic) -> EditorDiagnostic {
    let level = match found.severity {
        Severity::Error => DiagnosticLevel::Error,
        Severity::Warning => DiagnosticLevel::Warning,
    };
    EditorDiagnostic::new(found.span.clone(), level, found.message.clone())
        .with_code(found.code.as_str())
}

/// The findings of a pass so far (kept on the UI thread between chunks).
#[derive(Clone, Debug)]
pub(crate) struct Progress {
    version: u64,
    mode: LanguageMode,
    language: Language,
    diagnostics: Vec<EditorDiagnostic>,
    missing: Vec<Gvk>,
    coverage: Coverage,
}

impl Progress {
    /// A pass over `plan`: the buffer's syntax errors are known from the parse already.
    pub(crate) fn start(plan: &Plan) -> Self {
        let mut progress = Self {
            version: plan.version,
            mode: plan.mode,
            language: plan.language,
            diagnostics: plan.base.clone(),
            missing: Vec::new(),
            coverage: Coverage {
                done: 0,
                total: plan.chunks.len(),
            },
        };
        progress.finish_at_cap();
        progress
    }

    /// How many more diagnostics the pass may report before the cap.
    pub(crate) fn room(&self) -> usize {
        ValidateOptions::DEFAULT_MAX_DIAGNOSTICS.saturating_sub(self.diagnostics.len())
    }

    /// Adds the next chunk's findings. Once the cap is reached the pass is complete: the rest
    /// would not be shown.
    pub(crate) fn add(&mut self, findings: Findings) {
        self.coverage.done += 1;
        self.diagnostics.extend(findings.diagnostics);
        for gvk in findings.missing {
            if !self.missing.contains(&gvk) {
                self.missing.push(gvk);
            }
        }
        self.finish_at_cap();
    }

    /// The diagnostics cap ends the pass: what is left would not be shown.
    fn finish_at_cap(&mut self) {
        if self.room() == 0 {
            self.coverage.done = self.coverage.total;
        }
    }

    /// Whether every chunk is in.
    pub(crate) fn is_complete(&self) -> bool {
        self.coverage.is_complete()
    }

    /// The pass so far, to show: the diagnostics sorted by position.
    pub(crate) fn validation(&self) -> Validation {
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.sort_by_key(|d| (d.range.start, d.range.end));
        Validation {
            version: self.version,
            mode: self.mode,
            language: self.language,
            diagnostics,
            missing: self.missing.clone(),
            coverage: self.coverage,
        }
    }
}

#[cfg(test)]
mod tests;
