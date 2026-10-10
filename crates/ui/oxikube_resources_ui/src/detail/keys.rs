//! The detail view's keys (E07-U559): the `DetailDrawer` key context and the actions bound in it.
//!
//! With the drawer focused (Enter on a table row moves the focus into it), `escape` closes it and
//! returns to the table, `j` / `k` and the arrows step the table's selection to the next or
//! previous object and show it, and `1` to `5` switch Overview, YAML, Describe, Events and (for a
//! CustomResourceDefinition) Schema. The bindings are in the per-OS keymap files of
//! `oxikube_assets`, so users rebind them in `keymap.json`. On the YAML and Describe tabs `/` opens a
//! find field (E11-S06), `n` / `N` step through its matches and `escape` in the field closes it; while
//! the field has the focus the context says `Editing`, so the bare keys are text.
//!
//! These are view-local moves like the table's own cursor keys and the close button; what they
//! open (the next object) is the `resource::Open` command through the usual dispatcher.

use gpui::{Action, Context, WeakEntity, Window, actions};
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
        /// Opens the find field over the YAML or Describe text (`resource::Find`).
        Find,
        /// Goes to the next match of the find, wrapping (`resource::NextMatch`).
        NextMatch,
        /// Goes to the previous match of the find, wrapping (`resource::PreviousMatch`).
        PreviousMatch,
        /// Closes the find field, forgets the matches and returns to the detail.
        CloseFind,
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
    const KEY_CONTEXT: &'static str = contexts::DETAIL_DRAWER;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        let mount = match self.mount {
            Mount::Drawer => "drawer",
            Mount::Tab => "tab",
        };
        context.value("mount", mount);
        // A text field inside has the focus: bare keys are text. `finding`: the strip is open.
        context.flag_if(self.find.editing, contexts::EDITING);
        context.flag_if(self.find.open, "finding");
        context.value("kind", self.target.gvk.kind.to_string());
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

    pub(super) fn on_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.request_find(None, window, cx);
    }

    pub(super) fn on_next_match(&mut self, _: &NextMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.request_next_match(cx);
    }

    pub(super) fn on_previous_match(
        &mut self,
        _: &PreviousMatch,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_previous_match(cx);
    }

    pub(super) fn on_close_find(
        &mut self,
        _: &CloseFind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_find(window, cx);
    }

    pub(super) fn on_show_tab(&mut self, action: &ShowTab, _: &mut Window, cx: &mut Context<Self>) {
        let tab = usize::from(action.index)
            .checked_sub(1)
            .and_then(|i| DetailTab::for_kind(&self.target.gvk).get(i).copied());
        if let Some(tab) = tab {
            self.set_tab(tab, cx);
        }
    }

    /// Steps the table of the shown object's kind by `delta` rows from the row of the object the
    /// drawer shows (not from the table's own cursor); the table dispatches `resource::Open`
    /// for the row it lands on and the drawer shows that object. A pinned tab, a detail whose
    /// kind has no table open, and one whose object is not listed there (filtered out) do
    /// nothing.
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.mount != Mount::Drawer {
            return;
        }
        if let Some(table) = self.origin.as_ref().and_then(WeakEntity::upgrade) {
            let from = self.target.clone();
            table.update(cx, |table, cx| table.step_detail(&from, delta, cx));
        }
    }
}
