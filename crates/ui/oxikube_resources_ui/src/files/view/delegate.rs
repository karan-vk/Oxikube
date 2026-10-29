//! [`FilesDelegate`]: the table's data source over the [`FilesModel`], and the row menu.

use gpui::{App, IntoElement, ParentElement as _, Styled as _, WeakEntity, Window, div, px};
use oxikube_app::files::FileKind;
use oxikube_ui::layout::h_flex;
use oxikube_ui::menu::{PopupMenu, PopupMenuItem};
use oxikube_ui::table::{SortDirection, TableColumn, TableDelegate, TextCell};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, u};

use super::FilesView;
use crate::files::model::{FileColumn, FileSort, FilesModel};

/// The table's delegate. See the module docs.
pub struct FilesDelegate {
    /// The rows.
    pub(crate) model: FilesModel,
    view: WeakEntity<FilesView>,
}

impl FilesDelegate {
    pub(crate) fn new(view: WeakEntity<FilesView>) -> Self {
        Self {
            model: FilesModel::new(),
            view,
        }
    }
}

fn direction(sort: FileSort, column: FileColumn) -> SortDirection {
    match (sort.column == column, sort.descending) {
        (false, _) => SortDirection::Unsorted,
        (true, true) => SortDirection::Descending,
        (true, false) => SortDirection::Ascending,
    }
}

fn icon_for(kind: FileKind) -> IconName {
    match kind {
        FileKind::Directory => IconName::Folder,
        FileKind::Symlink => IconName::Link,
        FileKind::File | FileKind::Special => IconName::FileText,
    }
}

impl TableDelegate for FilesDelegate {
    fn columns_count(&self, _: &App) -> usize {
        FileColumn::ALL.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.model.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let column = FileColumn::at(col_ix).unwrap_or(FileColumn::Name);
        let out = TableColumn::new(column.key(), column.title())
            .width(px(column.width()))
            .min_width(px(50.))
            .movable(false)
            .sortable()
            .sorted(direction(self.model.sort(), column));
        if column == FileColumn::Size {
            out.right()
        } else {
            out
        }
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
        match FileColumn::at(col_ix) {
            Some(FileColumn::Name) => h_flex()
                .gap(u(px(6.)))
                .items_center()
                .child(Icon::new(icon_for(row.entry.kind)).size(u(px(14.))).color(
                    if row.entry.kind == FileKind::Directory {
                        colors.accent
                    } else {
                        colors.text_muted
                    },
                ))
                .child(row.name.clone())
                .into_any_element(),
            Some(column) => div().child(row.cell(column)).into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn text_cell(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut App,
    ) -> Option<TextCell> {
        // The name cell has an icon: `render_td` draws it. The others are plain text.
        match FileColumn::at(col_ix)? {
            FileColumn::Name => None,
            column => Some(TextCell::new(self.model.row(row_ix)?.cell(column))),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        direction: SortDirection,
        _: &mut Window,
        _: &mut App,
    ) {
        let sort = match (FileColumn::at(col_ix), direction) {
            (Some(column), SortDirection::Ascending) => FileSort {
                column,
                descending: false,
            },
            (Some(column), SortDirection::Descending) => FileSort {
                column,
                descending: true,
            },
            _ => FileSort::default(),
        };
        self.model.set_sort(sort);
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let text = if self.model.hidden() > 0 {
            format!("Only hidden files here ({})", self.model.hidden())
        } else {
            "This directory is empty".to_owned()
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.colors().text_muted)
            .child(text)
    }

    fn row_selected(&self, row_ix: usize, _: &App) -> bool {
        self.model.selected_index() == Some(row_ix)
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut App,
    ) -> PopupMenu {
        let Some(row) = self.model.row(row_ix) else {
            return menu;
        };
        // The entries act on the entry right-clicked, by value: the listing may be refreshed
        // while the menu is open.
        let entry = row.entry.clone();
        let view = self.view.clone();
        let item = |label: &'static str,
                    run: fn(
            &mut FilesView,
            oxikube_app::files::FileEntry,
            &mut Window,
            &mut gpui::Context<FilesView>,
        )| {
            let (view, entry) = (view.clone(), entry.clone());
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                view.update(cx, |view, cx| run(view, entry.clone(), window, cx))
                    .ok();
            })
        };
        let mut menu = menu;
        if entry.is_navigable() {
            menu = menu.item(item("Open", |view, entry, _, cx| {
                view.open_entry(&entry, cx)
            }));
        }
        menu = menu.item(item("Download…", |view, entry, _, cx| {
            view.download_entry(entry, false, cx);
        }));
        if entry.kind == FileKind::Directory {
            menu = menu.item(item("Download as .tar…", |view, entry, _, cx| {
                view.download_entry(entry, true, cx);
            }));
        }
        menu.separator()
            .item(item("Copy Path", |view, entry, _, cx| {
                view.copy_path(&entry, cx)
            }))
            .item(item("Upload Here…", |view, _, window, cx| {
                view.upload_clicked(window, cx)
            }))
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        match (self.model.row(row_ix), FileColumn::at(col_ix)) {
            (Some(row), Some(column)) => row.cell(column).to_string(),
            _ => String::new(),
        }
    }
}
