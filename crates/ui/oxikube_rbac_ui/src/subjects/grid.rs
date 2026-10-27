//! [`MatrixDelegate`]: the matrix table, one row per API group and resource, one column per verb.

use std::sync::Arc;

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, px,
};
use oxikube_app::rbac_subjects::{MatrixColumn, MatrixLine, SubjectFilter, SubjectMatrix};
use oxikube_domain::access::Access;
use oxikube_ui::table::{TableColumn, TableDelegate, TextCell};
use oxikube_ui::{ActiveTokens as _, u};

/// Columns before the verbs: API group, resource.
const LEADING: usize = 2;
/// The design width of the leading columns and of a verb column, at 100 % zoom.
const GROUP_WIDTH: f32 = 220.;
const RESOURCE_WIDTH: f32 = 220.;
const VERB_WIDTH: f32 = 78.;
const EXTRAS_WIDTH: f32 = 160.;

/// The marker of a cell: granted, limited to named objects, not granted.
pub fn mark(access: Access) -> &'static str {
    match access {
        Access::Granted => "\u{2713}",
        Access::Restricted => "~",
        Access::Denied | Access::Unknown => "\u{2013}",
    }
}

/// The column at `ix`: the two leading text columns are `None`, the rest follow
/// [`MatrixColumn::all`].
fn column_at(ix: usize) -> Option<MatrixColumn> {
    ix.checked_sub(2).and_then(|ix| MatrixColumn::all().nth(ix))
}

/// The matrix table's delegate.
#[derive(Default)]
pub struct MatrixDelegate {
    matrix: Option<Arc<SubjectMatrix>>,
    /// The rows shown: indices into the matrix's lines, after the filter.
    shown: Vec<usize>,
    selected: Option<usize>,
    column: Option<MatrixColumn>,
    note: SharedString,
}

impl MatrixDelegate {
    pub(super) fn set_matrix(
        &mut self,
        matrix: Option<Arc<SubjectMatrix>>,
        filter: &SubjectFilter,
    ) {
        self.matrix = matrix;
        self.reshape(filter);
    }

    /// Applies `filter` to the group and resource of each line.
    pub(super) fn reshape(&mut self, filter: &SubjectFilter) {
        self.shown = match &self.matrix {
            Some(matrix) => matrix
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| filter.matches(&[line.group_label(), line.resource.as_str()]))
                .map(|(ix, _)| ix)
                .collect(),
            None => Vec::new(),
        };
        // The selection is a line: keep it when the filter still shows it, else drop it.
        if let Some(line) = self.selected
            && !self.shown.contains(&line)
        {
            self.selected = None;
        }
    }

    pub(super) fn set_note(&mut self, note: impl Into<SharedString>) {
        self.note = note.into();
    }

    pub(super) fn set_column(&mut self, column: Option<MatrixColumn>) {
        self.column = column;
    }

    /// The matrix the table shows.
    pub fn matrix(&self) -> Option<&Arc<SubjectMatrix>> {
        self.matrix.as_ref()
    }

    /// Lines before the filter.
    pub fn total(&self) -> usize {
        self.matrix.as_ref().map_or(0, |m| m.lines.len())
    }

    /// Lines after the filter.
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    /// Whether no line is shown.
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The line on table row `row`.
    pub fn line(&self, row: usize) -> Option<&MatrixLine> {
        let ix = *self.shown.get(row)?;
        self.matrix.as_ref()?.lines.get(ix)
    }

    /// The table row of the selected line.
    pub fn selected_row(&self) -> Option<usize> {
        let line = self.selected?;
        self.shown.iter().position(|&ix| ix == line)
    }

    pub(super) fn select_row(&mut self, row: Option<usize>) {
        self.selected = row.and_then(|row| self.shown.get(row).copied());
    }

    /// `group/resource` of the shown lines, top to bottom.
    pub fn shown_keys(&self) -> Vec<String> {
        (0..self.shown.len())
            .filter_map(|row| self.line(row))
            .map(|line| format!("{}/{}", line.api_group, line.resource))
            .collect()
    }
}

impl TableDelegate for MatrixDelegate {
    fn columns_count(&self, _: &App) -> usize {
        LEADING + MatrixColumn::all().count()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.shown.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let (title, width) = match column_at(col_ix) {
            None if col_ix == 0 => ("API GROUP", GROUP_WIDTH),
            None => ("RESOURCE", RESOURCE_WIDTH),
            Some(MatrixColumn::Extras) => ("EXTRAS", EXTRAS_WIDTH),
            Some(column) => (column.header(), VERB_WIDTH),
        };
        let mut spec = TableColumn::new(title.to_lowercase(), title)
            .width(px(width))
            .min_width(px(50.))
            .movable(false);
        if matches!(column_at(col_ix), Some(MatrixColumn::Verb(_))) {
            spec = spec.center();
        }
        spec
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let colors = cx.colors();
        let Some(line) = self.line(row_ix) else {
            return div().into_any_element();
        };
        match column_at(col_ix) {
            Some(MatrixColumn::Verb(verb)) => {
                let access = line.cell(verb);
                let colour = match access {
                    Access::Granted => colors.success,
                    Access::Restricted => colors.warning,
                    Access::Denied | Access::Unknown => colors.text_muted,
                };
                let highlighted = self.column == Some(MatrixColumn::Verb(verb))
                    && self.selected_row() == Some(row_ix);
                div()
                    .w_full()
                    .text_center()
                    .text_color(colour)
                    .when(highlighted, |cell| cell.font_weight(gpui::FontWeight::BOLD))
                    .child(mark(access))
                    .into_any_element()
            }
            _ => div()
                .truncate()
                .child(SharedString::from(self.cell_text(row_ix, col_ix, cx)))
                .into_any_element(),
        }
    }

    fn text_cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<TextCell> {
        if matches!(column_at(col_ix), Some(MatrixColumn::Verb(_))) {
            return None;
        }
        Some(TextCell::new(SharedString::from(
            self.cell_text(row_ix, col_ix, cx),
        )))
    }

    fn loading(&self, _: &App) -> bool {
        false
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let message = if self.total() == 0 {
            self.note.clone()
        } else {
            "No row matches the filter.".into()
        };
        div()
            .id("rbac-matrix-empty")
            .debug_selector(|| "rbac-matrix-empty".to_owned())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(u(px(16.)))
            .text_color(colors.text_muted)
            .child(message)
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        self.selected_row() == Some(row_ix)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let Some(line) = self.line(row_ix) else {
            return String::new();
        };
        match column_at(col_ix) {
            None if col_ix == 0 => line.group_label().to_owned(),
            None => line.resource.clone(),
            Some(column) => line.cell_text(column),
        }
    }
}

use gpui::prelude::FluentBuilder as _;
