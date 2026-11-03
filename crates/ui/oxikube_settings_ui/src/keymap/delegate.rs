//! [`KeymapDelegate`]: the table of the keymap editor, drawn from a [`KeymapListModel`].

use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_keymap::{KeymapLayer, KeymapPlatform, KeymapRow, RowStatus};
use oxikube_ui::kbd::keycap;
use oxikube_ui::layout::h_flex;
use oxikube_ui::table::{TableColumn, TableDelegate};
use oxikube_ui::tooltip::Tooltip;
use oxikube_ui::{ActiveTokens as _, Colors, u};

use super::model::{EmptyState, KeymapListModel, RowKey};

/// The columns' titles, in order.
const TITLES: [&str; 4] = ["Action", "Context", "Binding", "Source"];
/// The design width of each column, at 100 % zoom.
const WIDTHS: [f32; 4] = [340., 220., 200., 240.];

/// The table's data source. The rows and filters are the model's; the delegate adds the
/// selection (by identity, so it follows its row through reloads and filters).
pub struct KeymapDelegate {
    model: KeymapListModel,
    selected: Option<RowKey>,
    platform: KeymapPlatform,
    user_file_name: SharedString,
}

impl KeymapDelegate {
    /// A delegate over `model`, naming the default keymap of `platform` and the user's file
    /// `user_file_name` in the source tooltips.
    pub fn new(
        model: KeymapListModel,
        platform: KeymapPlatform,
        user_file_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            model,
            selected: None,
            platform,
            user_file_name: user_file_name.into(),
        }
    }

    /// The rows and filters.
    pub fn model(&self) -> &KeymapListModel {
        &self.model
    }

    /// The rows and filters, to change.
    pub fn model_mut(&mut self) -> &mut KeymapListModel {
        &mut self.model
    }

    /// The selected row, when it is showing.
    pub fn selected(&self) -> Option<&KeymapRow> {
        let at = self.selected_position()?;
        self.model.row(at)
    }

    /// Where the selected row is among the rows showing.
    pub fn selected_position(&self) -> Option<usize> {
        self.model.position_of(self.selected.as_ref()?)
    }

    /// Select the `index`th row showing; `false` when there is none.
    pub fn select(&mut self, index: usize) -> bool {
        match self.model.row(index) {
            Some(row) => {
                self.selected = Some(RowKey::of(row));
                true
            }
            None => false,
        }
    }

    /// The file the source of `row` is: shown in the source cell's tooltip.
    pub fn source_file(&self, row: &KeymapRow) -> SharedString {
        match row.source {
            Some(KeymapLayer::Default) => default_file_name(self.platform).into(),
            Some(KeymapLayer::Vim) => "vim.json (the base keymap)".into(),
            Some(KeymapLayer::User) => self.user_file_name.clone(),
            None => "No binding in any keymap".into(),
        }
    }
}

/// The embedded default keymap of `platform`, by its file name.
pub fn default_file_name(platform: KeymapPlatform) -> &'static str {
    match platform {
        KeymapPlatform::MacOs => "default-macos.json",
        KeymapPlatform::Linux => "default-linux.json",
        KeymapPlatform::Windows => "default-windows.json",
    }
}

/// What the Source cell says for `row`.
pub fn source_label(row: &KeymapRow) -> &'static str {
    match (row.source, row.status) {
        (None, _) | (_, RowStatus::Unbound) => "Not bound",
        (Some(KeymapLayer::Default), RowStatus::Active) => "Default",
        (Some(KeymapLayer::Vim), RowStatus::Active) => "Vim",
        (Some(KeymapLayer::User), RowStatus::Active) => "Your keymap",
        (Some(KeymapLayer::Default), RowStatus::Overridden) => "Default (overridden)",
        (Some(KeymapLayer::Vim), RowStatus::Overridden) => "Vim (overridden)",
        (Some(KeymapLayer::User), RowStatus::Overridden) => "Your keymap (overridden)",
        (Some(KeymapLayer::Default), RowStatus::UnboundByUser) => "Default (unbound by you)",
        (Some(KeymapLayer::Vim), RowStatus::UnboundByUser) => "Vim (unbound by you)",
        (Some(KeymapLayer::User), RowStatus::UnboundByUser) => "Your keymap (unbound by you)",
    }
}

/// What the Context cell says for `row`: nothing for a command bound nowhere.
pub fn context_label(row: &KeymapRow) -> &str {
    match (row.status, row.context.as_deref()) {
        (RowStatus::Unbound, _) => "",
        (_, Some(context)) => context,
        (_, None) => "Everywhere",
    }
}

impl KeymapDelegate {
    fn row_color(row: &KeymapRow, colors: &Colors) -> gpui::Hsla {
        match row.status {
            RowStatus::Active => colors.text,
            RowStatus::Overridden | RowStatus::UnboundByUser => colors.text_disabled,
            RowStatus::Unbound => colors.text_muted,
        }
    }

    fn action_cell(row: &KeymapRow, colors: &Colors) -> AnyElement {
        h_flex()
            .gap(u(px(8.)))
            .items_baseline()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .max_w_2_3()
                    .truncate()
                    .text_color(Self::row_color(row, colors))
                    .child(SharedString::from(row.title.clone())),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(row.action.clone())),
            )
            .into_any_element()
    }

    fn binding_cell(row: &KeymapRow, colors: &Colors) -> AnyElement {
        if row.keystrokes.is_empty() {
            return div()
                .text_color(colors.text_muted)
                .child("Not bound")
                .into_any_element();
        }
        let mut cell = h_flex().gap(u(px(6.))).items_center();
        for stroke in &row.keystrokes {
            cell = match keycap(stroke) {
                Some(cap) => cell.child(cap),
                None => cell.child(SharedString::from(stroke.clone())),
            };
        }
        if let Some(data) = &row.data {
            cell = cell.child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(data.clone())),
            );
        }
        cell.into_any_element()
    }

    fn source_cell(&self, row_ix: usize, row: &KeymapRow, colors: &Colors) -> AnyElement {
        let file = self.source_file(row);
        div()
            .id(("keymap-source", row_ix))
            .truncate()
            .text_color(Self::row_color(row, colors))
            .child(source_label(row))
            .tooltip(move |window, cx| Tooltip::new(file.clone()).build(window, cx))
            .into_any_element()
    }
}

impl TableDelegate for KeymapDelegate {
    fn columns_count(&self, _: &App) -> usize {
        TITLES.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.model.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        TableColumn::new(TITLES[col_ix].to_lowercase(), TITLES[col_ix])
            .width(px(WIDTHS[col_ix]))
            .min_width(px(80.))
            .movable(false)
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let colors = cx.colors();
        let Some(row) = self.model.row(row_ix) else {
            return div().into_any_element();
        };
        match col_ix {
            0 => Self::action_cell(row, &colors),
            1 => div()
                .truncate()
                .text_color(if row.context.is_some() {
                    Self::row_color(row, &colors)
                } else {
                    colors.text_muted
                })
                .child(SharedString::from(context_label(row).to_owned()))
                .into_any_element(),
            2 => Self::binding_cell(row, &colors),
            _ => self.source_cell(row_ix, row, &colors),
        }
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.colors();
        let message = match self.model.empty_state() {
            EmptyState::NoKeymap => "The keymap is not loaded.".to_owned(),
            EmptyState::NoMatches | EmptyState::None => {
                if self.model.query_text().trim().is_empty() {
                    "No key bindings in this context.".to_owned()
                } else {
                    format!("No action matches \"{}\".", self.model.query_text().trim())
                }
            }
        };
        div()
            .id("keymap-empty")
            .debug_selector(|| "keymap-empty".to_owned())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .px(u(px(16.)))
            .text_color(colors.text_muted)
            .child(SharedString::from(message))
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        match (&self.selected, self.model.row(row_ix)) {
            (Some(key), Some(row)) => key.is(row),
            _ => false,
        }
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let Some(row) = self.model.row(row_ix) else {
            return String::new();
        };
        match col_ix {
            0 => row.title.clone(),
            1 => context_label(row).to_owned(),
            2 => row.keystrokes_text(),
            _ => source_label(row).to_owned(),
        }
    }
}
