//! [`PanelDelegate`]: the table's data source over the [`PanelModel`].
//!
//! Every cell is text built once per refresh (see [`Row`]); the status cell adds an icon in the
//! theme's status colour. The selection is the model's: it follows the forward, not the line, and
//! the table only reports clicks.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div, px,
};
use oxikube_ui::table::{TableColumn, TableDelegate, TextCell};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, u};

use crate::model::{PanelModel, Row, StatusKind};

/// The columns, in order.
pub(super) const COLUMNS: [(&str, &str, f32); 6] = [
    ("cluster", "Cluster", 140.),
    ("target", "Target", 230.),
    ("ports", "Ports (local -> remote)", 170.),
    ("status", "Status", 300.),
    ("traffic", "Traffic", 200.),
    ("uptime", "Uptime", 80.),
];

const STATUS: usize = 3;

/// The table's delegate. See the module docs.
pub struct PanelDelegate {
    /// The rows, the filters and the selection.
    pub(super) model: PanelModel,
}

impl PanelDelegate {
    pub(super) fn new() -> Self {
        Self {
            model: PanelModel::new(),
        }
    }
}

fn text(row: &Row, col_ix: usize) -> Option<&gpui::SharedString> {
    Some(match col_ix {
        0 => &row.cluster_label,
        1 => &row.target,
        2 => &row.ports,
        3 => &row.status,
        4 => &row.traffic,
        5 => &row.uptime,
        _ => return None,
    })
}

/// The icon of a status.
pub(super) fn status_icon(kind: StatusKind) -> IconName {
    match kind {
        StatusKind::Active => IconName::CircleCheck,
        StatusKind::Starting => IconName::LoaderCircle,
        StatusKind::Reconnecting => IconName::RefreshCw,
        StatusKind::Failed => IconName::CircleX,
        StatusKind::Stopped => IconName::Circle,
    }
}

impl TableDelegate for PanelDelegate {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.model.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let (key, name, width) = COLUMNS.get(col_ix).copied().unwrap_or(COLUMNS[0]);
        TableColumn::new(key, name)
            .width(px(width))
            .min_width(px(60.))
            .movable(false)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let Some(row) = self.model.row(row_ix) else {
            return div().into_any_element();
        };
        let label = text(row, col_ix).cloned().unwrap_or_default();
        if col_ix != STATUS {
            return div().child(label).into_any_element();
        }
        let colors = cx.colors();
        let colour = match row.kind {
            StatusKind::Active => colors.success,
            StatusKind::Starting | StatusKind::Reconnecting => colors.warning,
            StatusKind::Failed => colors.error,
            StatusKind::Stopped => colors.text_muted,
        };
        h_flex()
            .gap(u(px(6.)))
            .items_center()
            .text_color(colour)
            .child(
                Icon::new(status_icon(row.kind))
                    .size(u(px(12.)))
                    .color(colour),
            )
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
        let row = self.model.row(row_ix)?;
        Some(TextCell::new(text(row, col_ix)?.clone()))
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let message = if self.model.total() == 0 {
            "No port-forwards. Select a pod and press shift-f, or open a Service's port, to \
             start one."
        } else {
            "No port-forward matches the filter."
        };
        div()
            .id("port-forwards-empty")
            .debug_selector(|| "port-forwards-empty".to_owned())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(u(px(16.)))
            .text_color(colors.text_muted)
            .child(message)
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        self.model.is_selected(row_ix)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.model
            .row(row_ix)
            .and_then(|row| text(row, col_ix))
            .map(ToString::to_string)
            .unwrap_or_default()
    }
}
