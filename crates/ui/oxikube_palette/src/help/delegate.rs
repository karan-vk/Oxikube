//! [`HelpDelegate`]: the picker delegate behind the overlay.
//!
//! The picker supplies the search field, the keyboard selection and the virtualised list; the
//! delegate holds the [`HelpModel`] and the [`Row`]s of the current query. Group headers are
//! rows that cannot be selected.

use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, DismissEvent, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Task, Window, div,
};
use oxikube_ui::{ActiveTokens as _, u};

use super::model::{HelpModel, HelpScope, Row};
use super::render;
use crate::picker::fuzzy::{self, INLINE_MATCH_LIMIT, StringMatch};
use crate::picker::{Picker, PickerDelegate};

/// The overlay's delegate. Read-only: confirming a row closes the overlay and runs nothing.
pub struct HelpDelegate {
    model: Arc<HelpModel>,
    rows: Vec<Row>,
    selected: usize,
}

impl HelpDelegate {
    /// A delegate over `model`, showing every binding.
    pub fn new(model: Arc<HelpModel>) -> Self {
        let mut delegate = Self {
            model,
            rows: Vec::new(),
            selected: 0,
        };
        delegate.set_rows("", Vec::new());
        delegate
    }

    /// The model the delegate lists.
    pub fn model(&self) -> &HelpModel {
        &self.model
    }

    /// The rows of the current query.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn set_rows(&mut self, query: &str, matches: Vec<StringMatch>) {
        self.rows = if matches.is_empty() && query.trim().is_empty() {
            self.model.search("")
        } else {
            self.model.rows(query, &matches)
        };
        self.selected = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Entry { .. }))
            .unwrap_or(0);
    }
}

impl PickerDelegate for HelpDelegate {
    type ListItem = gpui::Stateful<gpui::Div>;

    fn match_count(&self) -> usize {
        self.rows.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.selected = ix;
    }

    fn can_select(&self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) -> bool {
        matches!(self.rows.get(ix), Some(Row::Entry { .. }))
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Search keys, commands or categories".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        Some(if self.model.entries().is_empty() {
            "No keys are bound where you are. Open a cluster to see its keys.".into()
        } else {
            "No keys match".into()
        })
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let candidates = self.model.candidates_for(&query);
        if candidates.len() <= INLINE_MATCH_LIMIT {
            // Well under a millisecond: the keystroke's rows are in this frame.
            let matches = fuzzy::match_strings(&candidates, &query, usize::MAX);
            self.set_rows(&query, matches);
            return Task::ready(());
        }
        cx.spawn_in(window, async move |picker, cx| {
            let executor = cx.background_executor().clone();
            let matches =
                fuzzy::match_strings_async(candidates, query.clone(), usize::MAX, &executor).await;
            picker
                .update(cx, |picker, _| picker.delegate.set_rows(&query, matches))
                .ok();
        })
    }

    fn confirm(&mut self, _: bool, _: &mut Window, cx: &mut Context<Picker<Self>>) {
        cx.emit(DismissEvent);
    }

    fn dismissed(&mut self, _: &mut Window, _: &mut Context<Picker<Self>>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        Some(match self.rows.get(ix)? {
            Row::Header { category, count } => render::header(*category, *count, ix, cx),
            Row::Entry { entry, positions } => render::entry(
                self.model.entries().get(*entry)?,
                positions,
                self.model.scope(),
                selected,
                ix,
                cx,
            ),
        })
    }

    fn render_header(&self, _: &mut Window, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let caption: SharedString = match self.model.scope() {
            HelpScope::Focused { innermost } => format!("Keys that work in {innermost}").into(),
            HelpScope::Everything => "Every key binding (no view has the focus)".into(),
        };
        Some(
            div()
                .debug_selector(|| "help-scope".to_owned())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child(caption)
                .into_any_element(),
        )
    }

    fn render_footer(&self, _: &mut Window, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        Some(
            div()
                .debug_selector(|| "help-footer".to_owned())
                .px(u(tokens.spacing.lg))
                .py(u(tokens.spacing.sm))
                .border_t_1()
                .border_color(tokens.colors.border_variant)
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text_muted)
                .child("Up and down to move, Esc to close. This list only shows keys; it runs nothing.")
                .into_any_element(),
        )
    }
}
