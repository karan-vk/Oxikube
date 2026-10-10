//! [`LiveEditor`]: [`EditorApi`] over a [`CodeEditor`], for the length of one borrow of the
//! window and the app.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Bounds, Context, Pixels, Window};
use gpui_component::input::{RangeDecoration, RangeDecorationStyle, TextDecoration};

use super::api::{Decoration, DecorationStyle, DiagnosticLevel, EditorApi, EditorDiagnostic};
use super::code_editor::CodeEditor;
use super::overlay::status_label;
use super::positions::{line_summaries, to_library};
use crate::tokens::ActiveTokens as _;

/// [`EditorApi`] over a [`CodeEditor`], borrowing the window and the app for one call sequence.
pub struct LiveEditor<'a, 'b> {
    editor: &'a mut CodeEditor,
    window: &'a mut Window,
    cx: &'a mut Context<'b, CodeEditor>,
}

impl<'a, 'b> LiveEditor<'a, 'b> {
    pub(super) fn new(
        editor: &'a mut CodeEditor,
        window: &'a mut Window,
        cx: &'a mut Context<'b, CodeEditor>,
    ) -> Self {
        Self { editor, window, cx }
    }
}

impl EditorApi for LiveEditor<'_, '_> {
    fn text(&self) -> Arc<str> {
        Arc::from(self.editor.state.read(self.cx).value().as_ref())
    }

    fn version(&self) -> u64 {
        self.editor.version
    }

    fn set_text(&mut self, text: &str) {
        super::set_text(&self.editor.state, text, self.window, self.cx);
        // `set_value` emits no change event: count it here.
        self.editor.changed(self.cx);
    }

    fn selections(&self) -> Vec<Range<usize>> {
        vec![self.editor.state.read(self.cx).selected_range()]
    }

    fn select(&mut self, range: Range<usize>) {
        self.editor
            .state
            .update(self.cx, |state, cx| state.set_selected_range(range, cx));
    }

    fn is_read_only(&self) -> bool {
        self.editor.read_only
    }

    fn set_read_only(&mut self, read_only: bool) {
        self.editor.read_only = read_only;
        self.editor
            .state
            .update(self.cx, |state, cx| state.set_readonly(read_only, cx));
        self.cx.notify();
    }

    fn soft_wrap(&self) -> bool {
        self.editor.soft_wrap
    }

    fn set_soft_wrap(&mut self, wrap: bool) {
        self.editor.soft_wrap = wrap;
        let window = &mut *self.window;
        self.editor
            .state
            .update(self.cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
        self.cx.notify();
    }

    fn set_diagnostics(&mut self, mut diagnostics: Vec<EditorDiagnostic>) {
        diagnostics.sort_by_key(|d| (d.range.start, d.range.end));
        let rope = self.editor.state.read(self.cx).text().clone();
        self.editor.state.update(self.cx, |state, cx| {
            if let Some(set) = state.diagnostics_mut() {
                set.reset(&rope);
                set.extend(diagnostics.iter().map(|d| to_library(&rope, d)));
            }
            cx.notify();
        });
        let count = |level| diagnostics.iter().filter(|d| d.level == level).count();
        let (errors, warnings) = (
            count(DiagnosticLevel::Error),
            count(DiagnosticLevel::Warning),
        );
        let summaries = line_summaries(&rope, &diagnostics);
        self.editor.status = status_label(&summaries, errors, warnings).into();
        self.editor.summaries = Rc::new(summaries);
        self.editor.diagnostics = diagnostics;
        self.cx.notify();
    }

    fn diagnostics(&self) -> Vec<EditorDiagnostic> {
        self.editor.diagnostics.clone()
    }

    fn add_decoration(&mut self, decoration: Decoration) {
        let Decoration { range, style } = decoration;
        match style {
            DecorationStyle::Dimmed => {
                let item = TextDecoration::new(
                    range,
                    gpui::HighlightStyle {
                        fade_out: Some(0.5),
                        ..Default::default()
                    },
                );
                match &self.editor.dimmed {
                    Some(collection) => collection.append(vec![item], self.cx),
                    None => {
                        let collection = self.editor.state.update(self.cx, |state, cx| {
                            state.create_decorations_collection(vec![item], cx)
                        });
                        self.editor.dimmed = Some(collection);
                    }
                }
            }
            DecorationStyle::Highlight | DecorationStyle::Frame => {
                let (geometry, slot) = if style == DecorationStyle::Highlight {
                    (RangeDecorationStyle::Fill, &mut self.editor.fills)
                } else {
                    (RangeDecorationStyle::Frame, &mut self.editor.frames)
                };
                let item = RangeDecoration::new(range)
                    .with_style(geometry)
                    .with_color(self.cx.colors().accent);
                match slot {
                    Some(collection) => collection.append(vec![item], self.cx),
                    None => {
                        *slot = Some(self.editor.state.update(self.cx, |state, cx| {
                            state.create_range_decorations_collection(vec![item], cx)
                        }));
                    }
                }
            }
        }
    }

    fn decorations(&self) -> Vec<Decoration> {
        let cx: &gpui::App = self.cx;
        let dimmed = self.editor.dimmed.iter().flat_map(|c| c.get_ranges(cx));
        let tag = |style| move |range| Decoration { range, style };
        let mut out: Vec<Decoration> = dimmed.map(tag(DecorationStyle::Dimmed)).collect();
        for (collection, style) in [
            (&self.editor.fills, DecorationStyle::Highlight),
            (&self.editor.frames, DecorationStyle::Frame),
        ] {
            out.extend(
                collection
                    .iter()
                    .flat_map(|c| c.get_ranges(cx))
                    .map(tag(style)),
            );
        }
        out
    }

    fn clear_decorations(&mut self) {
        if let Some(collection) = &self.editor.dimmed {
            collection.clear(self.cx);
        }
        for collection in [&self.editor.fills, &self.editor.frames]
            .into_iter()
            .flatten()
        {
            collection.clear(self.cx);
        }
    }

    fn range_to_bounds(&self, range: Range<usize>) -> Option<Bounds<Pixels>> {
        self.editor.state.read(self.cx).range_to_bounds(&range)
    }
}
