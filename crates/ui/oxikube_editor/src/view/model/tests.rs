//! The view logic against a fake editor: no window, no gpui context (E10-S04).

use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Pixels};
use oxikube_domain::OxiError;
use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_ui::editor::{Decoration, DiagnosticLevel, EditorApi, EditorDiagnostic};
use serde_json::json;

use super::{Accepted, ManifestModel, Problems};
use crate::view::validation::validate_text;

/// An editor that is a string and a few flags.
#[derive(Default)]
struct FakeEditor {
    text: String,
    version: u64,
    read_only: bool,
    soft_wrap: bool,
    diagnostics: Vec<EditorDiagnostic>,
    decorations: Vec<Decoration>,
    selection: Range<usize>,
}

impl FakeEditor {
    fn with_text(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            ..Self::default()
        }
    }

    /// Types `text` at the end, as a user would: a new version, the diagnostics dropped.
    fn type_text(&mut self, text: &str) {
        self.text.push_str(text);
        self.version += 1;
        self.diagnostics.clear();
    }
}

impl EditorApi for FakeEditor {
    fn text(&self) -> Arc<str> {
        Arc::from(self.text.as_str())
    }
    fn version(&self) -> u64 {
        self.version
    }
    fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.version += 1;
        self.diagnostics.clear();
    }
    fn selections(&self) -> Vec<Range<usize>> {
        vec![self.selection.clone()]
    }
    fn select(&mut self, range: Range<usize>) {
        self.selection = range;
    }
    fn is_read_only(&self) -> bool {
        self.read_only
    }
    fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }
    fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }
    fn set_soft_wrap(&mut self, wrap: bool) {
        self.soft_wrap = wrap;
    }
    fn set_diagnostics(&mut self, diagnostics: Vec<EditorDiagnostic>) {
        self.diagnostics = diagnostics;
    }
    fn diagnostics(&self) -> Vec<EditorDiagnostic> {
        self.diagnostics.clone()
    }
    fn add_decoration(&mut self, decoration: Decoration) {
        self.decorations.push(decoration);
    }
    fn decorations(&self) -> Vec<Decoration> {
        self.decorations.clone()
    }
    fn clear_decorations(&mut self) {
        self.decorations.clear();
    }
    fn range_to_bounds(&self, _: Range<usize>) -> Option<Bounds<Pixels>> {
        None
    }
}

fn configmap() -> Gvk {
    Gvk::new("", "v1", "ConfigMap")
}

fn configmap_schema() -> Arc<JsonSchema> {
    Arc::new(JsonSchema::from_value(&json!({
        "type": "object",
        "required": ["metadata"],
        "properties": {
            "apiVersion": {"type": "string"},
            "kind": {"type": "string"},
            "metadata": {"type": "object"},
            "data": {"type": "object", "additionalProperties": {"type": "string"}}
        }
    })))
}

const CONFIGMAP: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata: {}\ndata:\n  a: 1\n";

/// One full cycle as the view runs it: validate the current text, fetch what is missing, validate
/// again, accept.
fn validate(model: &mut ManifestModel, editor: &mut FakeEditor) -> Accepted {
    let result = validate_text(editor.version(), editor.text(), model.known());
    model.accept(editor, result)
}

#[test]
fn schema_findings_reach_the_editor_once_the_schema_arrives() {
    let mut editor = FakeEditor::with_text(CONFIGMAP);
    let mut model = ManifestModel::new();

    let first = validate_text(editor.version(), editor.text(), model.known());
    assert_eq!(first.missing, vec![configmap()]);
    assert_eq!(model.to_fetch(&first.missing), vec![configmap()]);
    // Asked again while the fetch runs: not fetched twice.
    assert!(model.to_fetch(&first.missing).is_empty());
    assert_eq!(model.accept(&mut editor, first), Accepted::Shown);
    assert!(
        editor.diagnostics.is_empty(),
        "no schema yet: nothing to say"
    );

    model.schema_arrived(configmap(), Ok(configmap_schema()));
    assert_eq!(validate(&mut model, &mut editor), Accepted::Shown);
    let mismatch = editor
        .diagnostics
        .iter()
        .find(|d| d.code.as_deref() == Some("type-mismatch"))
        .expect("`a: 1` is not a string");
    assert_eq!(&CONFIGMAP[mismatch.range.clone()], "1");
    assert_eq!(mismatch.level, DiagnosticLevel::Error);
    assert_eq!(
        model.problems(),
        Problems {
            errors: 1,
            warnings: 0
        }
    );
    assert_eq!(model.shown_version(), Some(editor.version()));
}

#[test]
fn a_result_for_an_older_version_is_dropped() {
    let mut editor = FakeEditor::with_text("a: [\n");
    let mut model = ManifestModel::new();
    let late = validate_text(editor.version(), editor.text(), model.known());
    editor.type_text("]\n");
    model.changed();
    assert_eq!(model.accept(&mut editor, late), Accepted::Stale);
    assert!(
        editor.diagnostics.is_empty(),
        "the stale syntax error is not shown"
    );
    assert_eq!(model.problems(), Problems::default());
    assert_eq!(model.shown_version(), None);

    assert_eq!(validate(&mut model, &mut editor), Accepted::Shown);
    assert!(editor.diagnostics.is_empty(), "`a: []` is fine");
}

#[test]
fn a_kind_the_cluster_lacks_is_known_as_schemaless_and_failures_are_kept() {
    let mut model = ManifestModel::new();
    let widget = Gvk::new("example.com", "v1", "Widget");
    assert_eq!(
        model.to_fetch(std::slice::from_ref(&widget)),
        vec![widget.clone()]
    );
    model.schema_arrived(widget.clone(), Err(OxiError::not_found("no schema")));
    assert!(model.to_fetch(std::slice::from_ref(&widget)).is_empty());
    assert!(model.unavailable().is_empty(), "not found is not a failure");

    model.to_fetch(&[configmap()]);
    model.schema_arrived(configmap(), Err(OxiError::network("connection refused")));
    assert_eq!(model.known().get(&configmap()), Some(&None));
    assert_eq!(model.unavailable().len(), 1);
    assert!(model.unavailable()[0].1.contains("connection refused"));
}

#[test]
fn syntax_errors_show_without_any_schema() {
    let mut editor = FakeEditor::with_text("kind: [\n");
    let mut model = ManifestModel::new();
    assert_eq!(validate(&mut model, &mut editor), Accepted::Shown);
    assert_eq!(model.problems().errors, editor.diagnostics.len());
    assert!(model.problems().errors >= 1);
}
