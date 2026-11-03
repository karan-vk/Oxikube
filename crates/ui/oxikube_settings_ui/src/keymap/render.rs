//! Drawing the keymap editor: the toolbar, the table and the hint under it.

use gpui::{
    AnyElement, Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, div, px,
};
use oxikube_domain::command::Command;
use oxikube_ui::button::Button;
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};
use oxikube_ui::{ActiveTokens as _, Sizable as _, Table, u};

use super::model::ContextFilter;
use super::view::KeymapEditorView;

impl Render for KeymapEditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let hint = self.user_hint(cx);
        v_flex()
            .id("keymap-editor")
            .debug_selector(|| "keymap-editor".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                // Plain keys only: a chord belongs to the keymap. Enter in the search box is the
                // input's, so it never opens the file.
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let in_search = this.search.read(cx).focus_handle(cx).is_focused(window);
                match event.keystroke.key.as_str() {
                    "down" => this.move_selection(1, cx),
                    "up" => this.move_selection(-1, cx),
                    "enter" if !in_search => this.open_json(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(u(tokens.font.body))
            .child(self.header(cx))
            .child(
                div()
                    .id("keymap-table")
                    .debug_selector(|| "keymap-table".into())
                    .flex_1()
                    .min_h(u(px(120.)))
                    .w_full()
                    .child(Table::new(&self.table).bordered(false)),
            )
            .children(hint)
    }
}

impl KeymapEditorView {
    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let (shown, total, show_unbound) = self.table.read(cx, |d| {
            (d.model().len(), d.model().total(), d.model().show_unbound())
        });
        let view = cx.entity().downgrade();
        v_flex()
            .flex_none()
            .gap(u(tokens.spacing.md))
            .px(u(tokens.spacing.xl))
            .py(u(tokens.spacing.lg))
            .border_b_1()
            .border_color(colors.border_variant)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.lg))
                    .items_center()
                    .child(
                        div()
                            .text_size(u(tokens.font.heading))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Keymap"),
                    )
                    .child(
                        div()
                            .id("keymap-count")
                            .debug_selector(|| "keymap-count".into())
                            .flex_1()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!("{shown} of {total}"))),
                    )
                    .child(self.edit_buttons(cx))
                    .child(
                        div().debug_selector(|| "keymap-open-json".into()).child(
                            Button::new("keymap-open-json")
                                .label("Open keymap.json")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.open_json(cx))),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .gap(u(tokens.spacing.lg))
                    .items_center()
                    .child(
                        div()
                            .id("keymap-search")
                            .debug_selector(|| "keymap-search".into())
                            .flex_1()
                            .child(Input::new(&self.search).cleanable(true)),
                    )
                    .child(self.context_menu(cx))
                    .child(
                        Checkbox::new("keymap-show-unbound")
                            .label("Show unbound")
                            .checked(show_unbound)
                            .on_click(move |checked, _, cx| {
                                view.update(cx, |this, cx| this.set_show_unbound(*checked, cx))
                                    .ok();
                            }),
                    ),
            )
            .into_any_element()
    }

    /// "Record binding" (`keymap::RecordBinding`, the recorder on the selected row) and "Remove
    /// binding" (`keymap::RemoveBinding`), enabled by the selection (E21-S08).
    fn edit_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let selected = self.selected_row(cx).is_some();
        let removable = self.can_remove_selected(cx);
        h_flex()
            .gap(u(tokens.spacing.md))
            .child(
                div().debug_selector(|| "keymap-record".into()).child(
                    Button::new("keymap-record")
                        .label("Record binding")
                        .small()
                        .disabled(!selected)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.deps
                                .dispatcher
                                .dispatch(Command::KeymapRecordBinding, cx);
                        })),
                ),
            )
            .child(
                div().debug_selector(|| "keymap-remove".into()).child(
                    Button::new("keymap-remove")
                        .label("Remove binding")
                        .small()
                        .disabled(!removable)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.remove_selected(cx);
                        })),
                ),
            )
            .into_any_element()
    }

    /// The context filter: the current choice on the button, every context with bindings in the
    /// menu (scrollable: a build has dozens).
    fn context_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let (current, contexts, everywhere) = self.table.read(cx, |d| {
            (
                d.model().context().clone(),
                d.model().contexts().to_vec(),
                d.model().has_everywhere(),
            )
        });
        let mut choices = vec![ContextFilter::All];
        if everywhere {
            choices.push(ContextFilter::Everywhere);
        }
        choices.extend(contexts.into_iter().map(ContextFilter::Named));
        let view = cx.entity().downgrade();
        let button = Button::new("keymap-context")
            .label(SharedString::from(current.label()))
            .small()
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu.scrollable(true).max_h(px(360.));
                for choice in &choices {
                    let (view, picked) = (view.clone(), choice.clone());
                    menu = menu.item(
                        PopupMenuItem::new(SharedString::from(choice.label()))
                            .checked(*choice == current)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.set_context(picked.clone(), cx))
                                    .ok();
                            }),
                    );
                }
                menu
            });
        div()
            .flex_none()
            .debug_selector(|| "keymap-context".into())
            .child(button)
            .into_any_element()
    }

    /// The line under the table that points at `keymap.json` while the user has bound nothing.
    fn user_hint(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.table.read(cx, |d| {
            d.model().has_user_bindings() || d.model().total() == 0
        }) {
            return None;
        }
        let tokens = cx.tokens();
        let colors = tokens.colors;
        Some(
            div()
                .id("keymap-no-user")
                .debug_selector(|| "keymap-no-user".into())
                .flex_none()
                .px(u(tokens.spacing.xl))
                .py(u(tokens.spacing.md))
                .border_t_1()
                .border_color(colors.border_variant)
                .text_color(colors.text_muted)
                .child(
                    "You have not changed any keys yet. Open keymap.json to bind or unbind one; \
                     the list updates when you save.",
                )
                .into_any_element(),
        )
    }
}
