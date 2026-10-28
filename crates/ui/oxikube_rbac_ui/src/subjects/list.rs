//! [`ListDelegate`]: the Users and Groups tables' rows, filter and selection.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, px,
};
use oxikube_app::rbac_subjects::{SubjectFilter, SubjectSummary};
use oxikube_domain::rbac::Subject;
use oxikube_ui::table::{TableColumn, TableDelegate, TextCell};
use oxikube_ui::{ActiveTokens as _, u};

/// The columns, in order.
const TITLES: [&str; 4] = ["Name", "Kind", "Bindings", "Roles"];
/// The design width of each column, at 100 % zoom.
const WIDTHS: [f32; 4] = [420., 140., 90., 90.];

struct Row {
    summary: SubjectSummary,
    cells: [SharedString; 4],
}

/// The delegate of the list table. Cell texts are built once per refresh; drawing reads them.
#[derive(Default)]
pub struct ListDelegate {
    rows: Vec<Row>,
    /// The rows shown: indices into `rows`, after the filter.
    shown: Vec<usize>,
    selected: Option<Subject>,
    ready: bool,
    empty_note: SharedString,
}

impl ListDelegate {
    /// Replaces the rows; the filter is applied again by the caller ([`reshape`](Self::reshape)).
    pub(super) fn set_rows(&mut self, summaries: Vec<SubjectSummary>, filter: &SubjectFilter) {
        self.rows = summaries
            .into_iter()
            .map(|summary| Row {
                cells: [
                    summary.name().to_string().into(),
                    summary.subject.kind().into(),
                    summary.bindings.to_string().into(),
                    summary.roles.to_string().into(),
                ],
                summary,
            })
            .collect();
        self.ready = true;
        self.reshape(filter);
    }

    /// Applies `filter` to the rows (one pass over the cell texts).
    pub(super) fn reshape(&mut self, filter: &SubjectFilter) {
        self.shown = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                filter.matches(&[&row.cells[0], &row.cells[1]].map(|cell| cell.as_ref()))
            })
            .map(|(ix, _)| ix)
            .collect();
    }

    pub(super) fn set_empty_note(&mut self, note: impl Into<SharedString>) {
        self.empty_note = note.into();
    }

    /// Rows before the filter.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// Rows after the filter.
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    /// Whether no row is shown.
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The subject on row `row`.
    pub fn subject(&self, row: usize) -> Option<&Subject> {
        let ix = *self.shown.get(row)?;
        Some(&self.rows.get(ix)?.summary.subject)
    }

    /// The row summary on row `row`.
    pub fn summary(&self, row: usize) -> Option<&SubjectSummary> {
        let ix = *self.shown.get(row)?;
        self.rows.get(ix).map(|row| &row.summary)
    }

    /// The row showing `subject`, if the filter lets it through.
    pub fn position_of(&self, subject: &Subject) -> Option<usize> {
        self.shown.iter().position(|&ix| {
            self.rows
                .get(ix)
                .is_some_and(|r| r.summary.subject == *subject)
        })
    }

    /// The selected subject (by identity: it follows the subject through refreshes and filters).
    pub fn selected(&self) -> Option<&Subject> {
        self.selected.as_ref()
    }

    pub(super) fn select(&mut self, subject: Option<Subject>) {
        self.selected = subject;
    }

    /// The names of the shown rows, top to bottom.
    pub fn shown_names(&self) -> Vec<String> {
        self.shown
            .iter()
            .filter_map(|&ix| self.rows.get(ix))
            .map(|row| row.cells[0].to_string())
            .collect()
    }

    fn cell(&self, row_ix: usize, col_ix: usize) -> Option<&SharedString> {
        let ix = *self.shown.get(row_ix)?;
        self.rows.get(ix)?.cells.get(col_ix)
    }
}

impl TableDelegate for ListDelegate {
    fn columns_count(&self, _: &App) -> usize {
        TITLES.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.shown.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let mut column = TableColumn::new(TITLES[col_ix].to_lowercase(), TITLES[col_ix])
            .width(px(WIDTHS[col_ix]))
            .min_width(px(60.))
            .movable(false);
        if col_ix >= 2 {
            column = column.right();
        }
        column
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut App,
    ) -> impl IntoElement {
        div()
            .truncate()
            .child(self.cell(row_ix, col_ix).cloned().unwrap_or_default())
    }

    fn text_cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut App,
    ) -> Option<TextCell> {
        Some(TextCell::new(self.cell(row_ix, col_ix)?.clone()))
    }

    fn loading(&self, _: &App) -> bool {
        !self.ready
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let message = if self.rows.is_empty() {
            self.empty_note.clone()
        } else {
            "No subject matches the filter.".into()
        };
        div()
            .id("rbac-subjects-empty")
            .debug_selector(|| "rbac-subjects-empty".to_owned())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(u(px(16.)))
            .text_color(colors.text_muted)
            .child(message)
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        match (&self.selected, self.subject(row_ix)) {
            (Some(selected), Some(subject)) => selected == subject,
            _ => false,
        }
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.cell(row_ix, col_ix)
            .map(ToString::to_string)
            .unwrap_or_default()
    }
}
