//! Drawing a resource table: a thin toolbar (kind, counts, the column picker) above the
//! virtualised table.
//!
//! Render does no data work: the rows were applied when the feed delivered and the cells are
//! read by the table for the visible rows only. The one per-frame write is "now" and the tone
//! colours into the delegate, which every visible cell reads.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};
use jiff::Timestamp;
use oxikube_app::ColumnId;
use oxikube_keymap::KeyContextual as _;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Table, u};

use super::cells::ToneColors;
use super::states::stale_badge;
use super::view::ResourceTable;

impl Render for ResourceTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let colors = ToneColors::current(cx);
        let (rows, selected, state) = self.table.update_quiet(cx, |d| {
            d.now = Timestamp::now();
            d.colors = Some(colors);
            d.cells.begin_frame(d.now, &d.provider);
            (d.rows.len(), d.selection.len(), d.table_state())
        });
        let tokens = cx.colors();
        let count: SharedString = if selected > 0 {
            format!("{selected} of {rows} selected").into()
        } else if !self.filter_parts.is_empty() {
            // The filter bar says "3 of 7".
            SharedString::default()
        } else {
            format!("{rows}").into()
        };
        let toolbar =
            h_flex()
                .id("resource-table-toolbar")
                .flex_none()
                .w_full()
                .h(u(px(32.)))
                .px(u(px(8.)))
                .gap(u(px(8.)))
                .items_center()
                .border_b_1()
                .border_color(tokens.border_variant)
                .child(
                    div()
                        .text_color(tokens.text)
                        .text_size(u(px(13.)))
                        .child(self.title.clone()),
                )
                .child(
                    div()
                        .debug_selector(|| "resource-table-count".into())
                        .text_color(tokens.text_muted)
                        .text_size(u(px(12.)))
                        .child(count),
                )
                .children(state.stale().map(|stale| {
                    stale_badge(stale, state.can_retry(), &cx.entity().downgrade(), cx)
                }))
                .child(self.filter.clone())
                .children(self.basic_columns_note(cx))
                .child(div().flex_1())
                .children(self.version_switcher(cx))
                .child(self.column_picker(cx));
        v_flex()
            .id("resource-table")
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.background)
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_extend_next))
            .on_action(cx.listener(Self::on_extend_previous))
            .on_action(cx.listener(Self::on_select_first))
            .on_action(cx.listener(Self::on_select_last))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_copy_name))
            .on_action(cx.listener(Self::on_select_all))
            .on_action(cx.listener(Self::on_clear))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_shell))
            .on_action(cx.listener(Self::on_attach))
            .on_action(cx.listener(Self::on_debug))
            .on_action(cx.listener(Self::on_focus_filter))
            .on_action(cx.listener(Self::on_clear_filter))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(Table::new(&self.table).bordered(false)),
            )
    }
}

impl ResourceTable {
    /// The "Columns" button: every column of the kind, checked when shown; clicking one shows or
    /// hides it. The name column cannot be hidden.
    fn column_picker(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let view = cx.entity().downgrade();
        Button::new("resource-table-columns")
            .label("Columns")
            .ghost()
            .xsmall()
            .dropdown_menu(move |mut menu, _, cx| {
                let Some(view) = view.upgrade() else {
                    return menu;
                };
                let columns: Vec<(ColumnId, String, bool)> = view.read(cx).read_rows(cx, |d| {
                    d.layout
                        .columns()
                        .map(|(c, shown)| (c.id.clone(), c.title.to_string(), shown))
                        .collect()
                });
                for (id, title, shown) in columns {
                    let target = view.downgrade();
                    let locked = id == ColumnId::NAME;
                    menu = menu.item(
                        PopupMenuItem::new(title)
                            .checked(shown)
                            .disabled(locked)
                            .on_click(move |_, _, cx| {
                                target
                                    .update(cx, |table, cx| table.set_column_shown(&id, !shown, cx))
                                    .ok();
                            }),
                    );
                }
                menu
            })
    }
}
