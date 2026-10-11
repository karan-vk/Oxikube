//! The view logic against a fake editor: no window, no gpui context (E10-S04).

use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Pixels};
use oxikube_domain::OxiError;
use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::JsonSchema;
use oxikube_ui::editor::{Decoration, DiagnosticLevel, EditorApi, EditorDiagnostic, EditorFont};
use serde_json::json;

use super::{Accepted, ManifestModel, Problems};
use crate::json::{Language, LanguageMode};
use crate::settings::{EditorSettings, apply_changes};
use crate::view::validation::validate_text_as;

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
    tab_size: usize,
    font: EditorFont,
    /// The setters called, in order, to see what `apply_changes` touched.
    calls: Vec<&'static str>,
    language: &'static str,
}

impl FakeEditor {
    fn with_text(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            language: "yaml",
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
    fn replace_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.version += 1;
        self.diagnostics.clear();
    }
    fn language(&self) -> &'static str {
        self.language
    }
    fn set_language(&mut self, language: &'static str) {
        self.language = language;
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
        self.calls.push("soft_wrap");
        self.soft_wrap = wrap;
    }
    fn tab_size(&self) -> usize {
        self.tab_size
    }
    fn set_tab_size(&mut self, spaces: usize) {
        self.calls.push("tab_size");
        self.tab_size = spaces;
    }
    fn font(&self) -> EditorFont {
        self.font.clone()
    }
    fn set_font(&mut self, font: EditorFont) {
        self.calls.push("font");
        self.font = font;
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
    let result = validate_text_as(editor.version(), editor.text(), model.known(), model.mode());
    model.accept(editor, result)
}

#[test]
fn schema_findings_reach_the_editor_once_the_schema_arrives() {
    let mut editor = FakeEditor::with_text(CONFIGMAP);
    let mut model = ManifestModel::new();

    let first = validate_text_as(editor.version(), editor.text(), model.known(), model.mode());
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
    let late = validate_text_as(editor.version(), editor.text(), model.known(), model.mode());
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

#[test]
fn clearing_drops_what_is_shown_and_costs_nothing_when_nothing_is() {
    let mut editor = FakeEditor::with_text("kind: [\n");
    let mut model = ManifestModel::new();
    validate(&mut model, &mut editor);
    assert!(!editor.diagnostics.is_empty());
    model.clear(&mut editor);
    assert!(editor.diagnostics.is_empty());
    assert_eq!(model.problems(), Problems::default());
    assert_eq!(model.shown_version(), None);
    // Typing with validation off: the editor dropped its squiggles itself; no second clear.
    editor.diagnostics = vec![EditorDiagnostic::new(0..1, DiagnosticLevel::Error, "kept")];
    model.clear(&mut editor);
    assert_eq!(
        editor.diagnostics.len(),
        1,
        "nothing was shown, nothing is cleared"
    );
}

#[test]
fn applying_settings_touches_only_what_changed_and_never_the_text() {
    let defaults = EditorSettings::default();
    let mut editor = FakeEditor::with_text("a: 1\n");
    editor.type_text("b: 2\n");
    let version = editor.version();

    apply_changes(None, &defaults, &mut editor);
    assert_eq!(
        editor.calls,
        ["soft_wrap", "tab_size", "font"],
        "a new editor gets all"
    );

    editor.calls.clear();
    let wider = EditorSettings {
        tab_size: 8,
        ..defaults.clone()
    };
    apply_changes(Some(&defaults), &wider, &mut editor);
    assert_eq!(editor.calls, ["tab_size"]);
    assert_eq!(editor.tab_size, 8);

    editor.calls.clear();
    let styled = EditorSettings {
        soft_wrap: true,
        font_family: Some("Menlo".into()),
        font_size: Some(14.0),
        ..wider.clone()
    };
    apply_changes(Some(&wider), &styled, &mut editor);
    assert_eq!(editor.calls, ["soft_wrap", "font"]);
    assert!(editor.soft_wrap);
    assert_eq!(editor.font, styled.font());

    // The settings that are not about the look change nothing in the editor.
    editor.calls.clear();
    let other = EditorSettings {
        validation: false,
        mask_secrets: false,
        field_manager: "other".into(),
        ..styled.clone()
    };
    apply_changes(Some(&styled), &other, &mut editor);
    assert!(editor.calls.is_empty());
    assert_eq!(editor.version(), version, "the buffer was not touched");
    assert_eq!(&*editor.text(), "a: 1\nb: 2\n");
}

#[test]
fn auto_mode_follows_the_content_and_tells_the_editor() {
    let mut editor = FakeEditor::with_text("kind: Pod\n");
    let mut model = ManifestModel::new();
    assert_eq!(model.mode(), LanguageMode::Auto);
    assert_eq!(validate(&mut model, &mut editor), Accepted::Shown);
    assert_eq!(model.language(), Language::Yaml);
    assert_eq!(editor.language, "yaml");

    editor.set_text("{\"kind\": \"Pod\"}\n");
    model.changed();
    assert_eq!(validate(&mut model, &mut editor), Accepted::Shown);
    assert_eq!(model.language(), Language::Json);
    assert_eq!(editor.language, "json", "highlighted with the JSON grammar");

    editor.set_text("kind: Pod\n");
    model.changed();
    validate(&mut model, &mut editor);
    assert_eq!(editor.language, "yaml", "and back");
}

#[test]
fn a_fixed_mode_beats_the_content() {
    let mut editor = FakeEditor::with_text("{\"a\": 1}\n");
    let mut model = ManifestModel::new();
    model.set_mode(LanguageMode::Fixed(Language::Yaml));
    assert_eq!(model.language(), Language::Yaml, "effective at once");
    validate(&mut model, &mut editor);
    assert_eq!(editor.language, "yaml");
    assert!(editor.diagnostics.is_empty(), "flow YAML is valid YAML");

    model.set_mode(LanguageMode::Fixed(Language::Json));
    editor.set_text("{\"a\": 1,}\n");
    model.changed();
    validate(&mut model, &mut editor);
    assert_eq!(editor.language, "json");
    assert_eq!(editor.diagnostics.len(), 1, "the trailing comma");
    assert_eq!(editor.diagnostics[0].code.as_deref(), Some("syntax"));
    assert_eq!(&editor.text[editor.diagnostics[0].range.clone()], ",");
}

#[test]
fn a_result_computed_under_another_mode_is_dropped() {
    let mut editor = FakeEditor::with_text("{\"a\": 1,}\n");
    let mut model = ManifestModel::new();
    model.set_mode(LanguageMode::Fixed(Language::Yaml));
    let late = validate_text_as(editor.version(), editor.text(), model.known(), model.mode());
    model.set_mode(LanguageMode::Fixed(Language::Json));
    assert_eq!(model.accept(&mut editor, late), Accepted::Stale);
    assert!(editor.diagnostics.is_empty());
    assert_eq!(model.shown_version(), None);
    assert_eq!(validate(&mut model, &mut editor), Accepted::Shown);
    assert_eq!(editor.diagnostics.len(), 1);
}
