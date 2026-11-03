//! The list editor: an array of strings as rows of inputs with add, remove and reorder.

use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Subscription, Window, div, px,
};
use oxikube_domain::ids::ClusterId;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::input::{Input, InputEvent, InputState};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};
use serde_json::Value;

use super::state::{Controls, Editor, effective_value};
use super::validate::validate_list;
use crate::page::{PageField, SettingsPage};

/// One row of a list editor.
pub(super) struct ListRow {
    pub input: Entity<InputState>,
    _subscription: Subscription,
}

/// The rows of one list field, in order.
pub struct ListRows {
    pub(super) rows: Vec<ListRow>,
}

impl ListRows {
    /// Rows for the items in effect for `field`.
    pub(super) fn new(
        field: &PageField,
        cluster: Option<&ClusterId>,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) -> Self {
        let mut rows = Self { rows: Vec::new() };
        rows.show(
            effective_value(&field.path, cluster, cx).as_ref(),
            field,
            window,
            cx,
        );
        rows
    }

    /// The text of every row.
    pub fn texts(&self, cx: &gpui::App) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.input.read(cx).value().to_string())
            .collect()
    }

    /// The texts as `validate_list` would write them: trimmed, blank rows left out.
    fn committed_texts(&self, cx: &gpui::App) -> Vec<String> {
        self.texts(cx)
            .into_iter()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect()
    }

    /// Replaces the rows by the items of `value` when they differ from what is shown.
    pub(super) fn show(
        &mut self,
        value: Option<&Value>,
        field: &PageField,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) {
        let items: Vec<String> = value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        // Compare what would be written (trimmed, blank rows left out), so a row still being
        // typed ("foo ") or a freshly added blank row is not replaced by the echo of a write.
        if self.committed_texts(cx) == items {
            return;
        }
        self.rows = items
            .into_iter()
            .map(|item| row(&field.path, item, window, cx))
            .collect();
    }
}

fn row(
    path: &SharedString,
    text: String,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> ListRow {
    let input = cx.new(|cx| {
        let mut state = InputState::new(window, cx);
        state.set_value(text, window, cx);
        state
    });
    let path = path.clone();
    let subscription =
        cx.subscribe_in(&input, window, move |page, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let cluster = page.cluster.clone();
                page.controls.list_changed(&path, cluster, false, cx);
            }
        });
    ListRow {
        input,
        _subscription: subscription,
    }
}

impl Controls {
    fn list_rows_mut(&mut self, path: &str) -> Option<&mut ListRows> {
        match self.editors.get_mut(path)? {
            Editor::List(rows) => Some(rows),
            Editor::Text { .. } => None,
        }
    }

    /// The rows of the list at `path`.
    pub(super) fn list_rows(&self, path: &str) -> Option<&ListRows> {
        match self.editors.get(path)? {
            Editor::List(rows) => Some(rows),
            Editor::Text { .. } => None,
        }
    }

    /// Appends a blank row. Nothing is written until it has text.
    pub fn add_item(
        &mut self,
        field: &PageField,
        window: &mut Window,
        cx: &mut Context<SettingsPage>,
    ) {
        if let Some(rows) = self.list_rows_mut(&field.path) {
            rows.rows.push(row(&field.path, String::new(), window, cx));
            cx.notify();
        }
    }

    /// Removes row `ix` and writes the rest at once.
    pub fn remove_item(
        &mut self,
        path: &SharedString,
        ix: usize,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        if let Some(rows) = self.list_rows_mut(path)
            && ix < rows.rows.len()
        {
            rows.rows.remove(ix);
            self.list_changed(path, cluster, true, cx);
        }
    }

    /// Moves row `ix` one place up (`up`) or down and writes the order at once.
    pub fn move_item(
        &mut self,
        path: &SharedString,
        ix: usize,
        up: bool,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        let Some(rows) = self.list_rows_mut(path) else {
            return;
        };
        let target = if up { ix.checked_sub(1) } else { Some(ix + 1) };
        let Some(target) =
            target.filter(|target| *target < rows.rows.len() && ix < rows.rows.len())
        else {
            return;
        };
        rows.rows.swap(ix, target);
        self.list_changed(path, cluster, true, cx);
    }

    pub(super) fn list_changed(
        &mut self,
        path: &SharedString,
        cluster: Option<ClusterId>,
        immediately: bool,
        cx: &mut Context<SettingsPage>,
    ) {
        let Some(items) = self.list_rows(path).map(|rows| rows.texts(cx)) else {
            return;
        };
        match validate_list(&items) {
            Ok(value) => self.accept(path, value, cluster, immediately, cx),
            Err(error) => self.reject(path, error, cx),
        }
    }
}

impl Controls {
    /// The list editor of `field`: one row per item, then "Add item".
    pub(super) fn list(
        &self,
        field: &PageField,
        cx: &mut Context<SettingsPage>,
    ) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let rows = self.list_rows(&field.path)?;
        let count = rows.rows.len();
        let items = rows.rows.iter().enumerate().map(|(ix, row)| {
            let path = field.path.clone();
            let button = |label: &'static str, name: &'static str| {
                Button::new(SharedString::from(format!(
                    "settings-list-{name}-{path}-{ix}"
                )))
                .xsmall()
                .ghost()
                .label(label)
            };
            let selector = |name: &'static str| {
                let path = path.clone();
                move || format!("settings-list-{name}-{path}-{ix}")
            };
            h_flex()
                .gap(u(tokens.spacing.sm))
                .child(
                    div()
                        .debug_selector(selector("item"))
                        .w(u(px(240.)))
                        .child(Input::new(&row.input).small()),
                )
                .child(div().debug_selector(selector("up")).child(
                    button("Up", "up").disabled(ix == 0).on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, _, cx| {
                            let cluster = this.cluster.clone();
                            this.controls.move_item(&path, ix, true, cluster, cx);
                        }
                    })),
                ))
                .child(
                    div().debug_selector(selector("down")).child(
                        button("Down", "down")
                            .disabled(ix + 1 >= count)
                            .on_click(cx.listener({
                                let path = path.clone();
                                move |this, _, _, cx| {
                                    let cluster = this.cluster.clone();
                                    this.controls.move_item(&path, ix, false, cluster, cx);
                                }
                            })),
                    ),
                )
                .child(div().debug_selector(selector("remove")).child(
                    button("Remove", "remove").on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, _, cx| {
                            let cluster = this.cluster.clone();
                            this.controls.remove_item(&path, ix, cluster, cx);
                        }
                    })),
                ))
        });
        let add_path = field.path.clone();
        let add_field = field.clone();
        Some(
            v_flex()
                .gap(u(tokens.spacing.xs))
                .children(items)
                .child(
                    div()
                        .debug_selector(move || format!("settings-list-add-{add_path}"))
                        .child(
                            Button::new(SharedString::from(format!(
                                "settings-list-add-{}",
                                field.path
                            )))
                            .xsmall()
                            .outline()
                            .label("Add item")
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.controls.add_item(&add_field, window, cx);
                                },
                            )),
                        ),
                )
                .into_any_element(),
        )
    }
}
