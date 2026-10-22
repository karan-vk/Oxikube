//! [`ReleasesDelegate`]: the table's data source over the shown rows.
//!
//! The rows are the release summaries the service listed; the delegate keeps the order and the
//! filter on top of them (indices into the list, sorted by [`sort_releases`]-compatible keys), and
//! the cell texts of each row, built once per refresh. Drawing reads those texts: nothing is
//! decoded or formatted per frame.

use std::sync::Arc;

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, px,
};
use jiff::Timestamp;
use oxikube_app::helm_releases::ReleaseFilter;
use oxikube_domain::helm::{Release, ReleaseColumn, ReleaseRef};
use oxikube_ui::table::{SortDirection, TableColumn, TableDelegate, TextCell};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, u};

use super::model::{Phase, RowText, Tone, tone};

/// The design width of each column, at 100 % zoom.
const WIDTHS: [f32; 8] = [200., 150., 150., 80., 100., 110., 150., 80.];

/// The table's delegate. See the module docs.
pub struct ReleasesDelegate {
    /// Every release listed, in the order the service gave.
    all: Arc<Vec<Release>>,
    texts: Vec<RowText>,
    /// The rows shown: indices into `all`, filtered and sorted.
    shown: Vec<usize>,
    filter: ReleaseFilter,
    sort: Option<(ReleaseColumn, bool)>,
    selected: Option<ReleaseRef>,
    phase: Phase,
}

impl ReleasesDelegate {
    pub(super) fn new() -> Self {
        Self {
            all: Arc::new(Vec::new()),
            texts: Vec::new(),
            shown: Vec::new(),
            filter: ReleaseFilter::default(),
            sort: None,
            selected: None,
            phase: Phase::Loading,
        }
    }

    /// Every release listed (before the filter).
    pub fn total(&self) -> usize {
        self.all.len()
    }

    /// How many rows the filter lets through.
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    /// Whether no row is shown.
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The release shown on row `row_ix`.
    pub fn release(&self, row_ix: usize) -> Option<&Release> {
        self.shown.get(row_ix).and_then(|&ix| self.all.get(ix))
    }

    /// The listed release `release`, whether or not the filter lets it through.
    pub fn find(&self, release: &ReleaseRef) -> Option<&Release> {
        self.all.iter().find(|r| r.release == *release)
    }

    /// The row showing `release`, if the filter lets it through.
    pub fn position_of(&self, release: &ReleaseRef) -> Option<usize> {
        self.shown
            .iter()
            .position(|&ix| self.all.get(ix).is_some_and(|r| r.release == *release))
    }

    /// The release the user selected (by identity: it follows the release through refreshes,
    /// sorts and filters).
    pub fn selected(&self) -> Option<&ReleaseRef> {
        self.selected.as_ref()
    }

    pub(super) fn select(&mut self, release: Option<ReleaseRef>) {
        self.selected = release;
    }

    /// The names of the shown rows, top to bottom (tests read them).
    pub fn shown_names(&self) -> Vec<String> {
        self.shown
            .iter()
            .filter_map(|&ix| self.all.get(ix))
            .map(|r| r.release.to_string())
            .collect()
    }

    /// The filter in effect.
    pub fn filter(&self) -> &ReleaseFilter {
        &self.filter
    }

    /// The sorted column and whether it is descending.
    pub fn sort(&self) -> Option<(ReleaseColumn, bool)> {
        self.sort
    }

    pub(super) fn phase(&self) -> &Phase {
        &self.phase
    }

    pub(super) fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    /// Replaces the listed releases; the filter and the sort stay.
    pub(super) fn set_releases(&mut self, releases: Vec<Release>, now: Timestamp) {
        self.texts = releases.iter().map(|r| RowText::new(r, now)).collect();
        self.all = Arc::new(releases);
        self.reshape();
    }

    /// Measures the ages again at `now`; whether any cell changed.
    pub(super) fn refresh_ages(&mut self, now: Timestamp) -> bool {
        let fresh: Vec<RowText> = self.all.iter().map(|r| RowText::new(r, now)).collect();
        let changed = fresh
            .iter()
            .zip(&self.texts)
            .any(|(new, old)| new.cell(ReleaseColumn::Updated) != old.cell(ReleaseColumn::Updated));
        if changed {
            self.texts = fresh;
        }
        changed
    }

    pub(super) fn set_filter(&mut self, filter: ReleaseFilter) {
        self.filter = filter;
        self.reshape();
    }

    pub(super) fn set_sort(&mut self, sort: Option<(ReleaseColumn, bool)>) {
        self.sort = sort;
        self.reshape();
    }

    /// Filters, then sorts: indices only, so a 500-row list costs one pass and one sort of
    /// integers.
    fn reshape(&mut self) {
        self.shown = self.filter.apply(&self.all);
        if let Some((column, descending)) = self.sort {
            let all = &self.all;
            self.shown.sort_by(|&a, &b| {
                let order = column.compare(&all[a], &all[b]);
                if descending { order.reverse() } else { order }
            });
        }
    }

    pub(super) fn text(&self, row_ix: usize, column: ReleaseColumn) -> Option<&SharedString> {
        let ix = *self.shown.get(row_ix)?;
        Some(self.texts.get(ix)?.cell(column))
    }
}

const STATUS: usize = ReleaseColumn::Status as usize;

fn column_at(col_ix: usize) -> ReleaseColumn {
    ReleaseColumn::ALL
        .get(col_ix)
        .copied()
        .unwrap_or(ReleaseColumn::Name)
}

/// The icon of a status tone.
pub(super) fn tone_icon(tone: Tone) -> IconName {
    match tone {
        Tone::Good => IconName::CircleCheck,
        Tone::Busy => IconName::LoaderCircle,
        Tone::Bad => IconName::CircleX,
        Tone::Quiet => IconName::Circle,
    }
}

/// The theme colour of a status tone.
pub(super) fn tone_colour(tone: Tone, cx: &App) -> gpui::Hsla {
    let colors = cx.colors();
    match tone {
        Tone::Good => colors.success,
        Tone::Busy => colors.warning,
        Tone::Bad => colors.error,
        Tone::Quiet => colors.text_muted,
    }
}

impl TableDelegate for ReleasesDelegate {
    fn columns_count(&self, _: &App) -> usize {
        ReleaseColumn::ALL.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.shown.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let column = column_at(col_ix);
        let direction = match self.sort {
            Some((sorted, false)) if sorted == column => SortDirection::Ascending,
            Some((sorted, true)) if sorted == column => SortDirection::Descending,
            _ => SortDirection::Unsorted,
        };
        let mut spec = TableColumn::new(
            column.title().to_lowercase().replace(' ', "-"),
            column.title(),
        )
        .width(px(WIDTHS[col_ix.min(WIDTHS.len() - 1)]))
        .min_width(px(60.))
        .movable(false)
        .sorted(direction);
        if matches!(column, ReleaseColumn::Revision | ReleaseColumn::Updated) {
            spec = spec.right();
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
        let Some(label) = self.text(row_ix, column_at(col_ix)).cloned() else {
            return div().into_any_element();
        };
        let Some(release) = self.release(row_ix) else {
            return div().into_any_element();
        };
        let tone = tone(release.status);
        let colour = tone_colour(tone, cx);
        h_flex()
            .gap(u(px(6.)))
            .items_center()
            .text_color(colour)
            .child(Icon::new(tone_icon(tone)).size(u(px(12.))).color(colour))
            .child(div().truncate().child(label))
            .into_any_element()
    }

    fn text_cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut App,
    ) -> Option<TextCell> {
        if col_ix == STATUS {
            return None;
        }
        Some(TextCell::new(self.text(row_ix, column_at(col_ix))?.clone()))
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        direction: SortDirection,
        _: &mut Window,
        _: &mut App,
    ) {
        let column = column_at(col_ix);
        self.set_sort(match direction {
            SortDirection::Unsorted => None,
            SortDirection::Ascending => Some((column, false)),
            SortDirection::Descending => Some((column, true)),
        });
    }

    fn loading(&self, _: &App) -> bool {
        matches!(self.phase, Phase::Loading) && self.all.is_empty()
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let message: SharedString = match &self.phase {
            Phase::Failed(why) => why.clone().into(),
            _ if self.all.is_empty() => {
                "No Helm releases in the namespaces you are watching.".into()
            }
            _ => "No release matches the filter.".into(),
        };
        let colour = if matches!(self.phase, Phase::Failed(_)) {
            colors.error
        } else {
            colors.text_muted
        };
        div()
            .id("helm-releases-empty")
            .debug_selector(|| "helm-releases-empty".to_owned())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(u(px(16.)))
            .text_color(colour)
            .child(message)
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        match (&self.selected, self.release(row_ix)) {
            (Some(selected), Some(release)) => release.release == *selected,
            _ => false,
        }
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.text(row_ix, column_at(col_ix))
            .map(ToString::to_string)
            .unwrap_or_default()
    }
}
