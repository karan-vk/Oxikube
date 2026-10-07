//! The detail view's keys (E07-U559): the `Detail` key context and the actions bound in it.
//!
//! With the drawer focused (Enter on a table row moves the focus into it), `escape` closes it and
//! returns to the table, `j` / `k` and the arrows step the table's selection to the next or
//! previous object and show it, and `1` to `5` switch Overview, YAML, Describe, Events and (for a
//! CustomResourceDefinition) Schema. The bindings are in the per-OS keymap files of
//! `oxikube_assets`, so users rebind them in `keymap.json`.
//!
//! These are view-local moves like the table's own cursor keys and the close button; what they
//! open (the next object) is the `resource::Open` command through the usual dispatcher.

use gpui::{Action, Context, Window, actions};
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use schemars::JsonSchema;
use serde::Deserialize;

use super::state::Mount;
use super::tabs::DetailTab;
use super::view::DetailView;

actions!(
    resource_detail,
    [
        /// Closes the drawer and returns to the table (a pinned detail tab ignores it).
        Close,
        /// Selects the next row of the table and shows its detail in the drawer.
        SelectNext,
        /// Selects the previous row of the table and shows its detail in the drawer.
        SelectPrevious,
    ]
);

/// Shows the nth tab of the detail, counted from 1 (Overview, YAML, Describe, Events, and a
/// CRD's Schema). A number the object has no tab for does nothing.
#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = resource_detail)]
pub struct ShowTab {
    /// The tab, from 1.
    pub index: u8,
}

impl KeyContextual for DetailView {
    const KEY_CONTEXT: &'static str = contexts::DETAIL;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        let mount = match self.mount {
            Mount::Drawer => "drawer",
            Mount::Tab => "tab",
        };
        context.value("mount", mount);
    }
}

impl DetailView {
    pub(super) fn on_close_action(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        if self.mount == Mount::Drawer {
            self.request_close(cx);
        }
    }

    pub(super) fn on_select_next(
        &mut self,
        _: &SelectNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step(1, cx);
    }

    pub(super) fn on_select_previous(
        &mut self,
        _: &SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step(-1, cx);
    }

    pub(super) fn on_show_tab(&mut self, action: &ShowTab, _: &mut Window, cx: &mut Context<Self>) {
        let tab = usize::from(action.index)
            .checked_sub(1)
            .and_then(|i| DetailTab::for_kind(&self.target.gvk).get(i).copied());
        if let Some(tab) = tab {
            self.set_tab(tab, cx);
        }
    }

    /// Steps the table the drawer was opened from by `delta` rows; the table dispatches
    /// `resource::Open` for the row it lands on and the drawer shows that object. A pinned tab
    /// and a detail without a table (an owner's, whose list is not open) do nothing.
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.mount != Mount::Drawer {
            return;
        }
        if let Some(table) = self.origin.as_ref().and_then(|table| table.upgrade()) {
            table.update(cx, |table, cx| table.step_detail(delta, cx));
        }
    }
}
