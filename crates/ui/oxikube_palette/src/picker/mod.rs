// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/picker/src/picker.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! [`Picker`]: a query field over a virtualised list of matches, driven by a [`PickerDelegate`].
//!
//! Zed's picker design: the picker owns the query field, the list, the selection keys, confirm
//! and dismissal; the delegate owns the data, filters it ([`PickerDelegate::update_matches`]),
//! renders one match and decides what confirming does. The command palette, the `:` jump bar's
//! completions, the container chooser of a pod shell and later pickers each only write a
//! delegate. It is presented through the workspace's modal layer (it implements [`ModalView`]):
//! `workspace.toggle_modal(window, cx, |window, cx| Picker::uniform_list(delegate, window, cx))`.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the struct, construction, the query and match updates |
//! | `delegate.rs` | [`PickerDelegate`], [`Direction`] |
//! | `selection.rs` | keyboard selection (wrapping, skipping unselectable matches), confirm, cancel, clicks |
//! | `render.rs` | the frame: query field, header, `uniform_list` of fixed-height rows, empty text, footer; [`match_label`] for delegates' rows |
//! | `actions.rs` | `picker::SelectNext` .. `picker::Cancel`, bound in the `Picker` key context |
//! | `fuzzy.rs` | [`fuzzy::match_strings`] and friends for delegates over strings |
//!
//! Adapted from Zed: the query field is `oxikube_ui`'s single-line input, rows and chrome use
//! `oxikube_ui` tokens, there is no preview, multi-select, resizing or persistence, and a match
//! update never clears its own task: a query generation says which update is current, so a slow
//! update for an old query neither reveals nor confirms anything. The picker never animates, so
//! reduce-motion has nothing to turn off.
//!
//! Keystrokes are not hardcoded: the per-OS keymap files bind them in the `Picker` and
//! `Picker > Input` contexts (`oxikube_keymap::contexts::PICKER`), so `keymap.json` rebinds them.

mod actions;
mod delegate;
pub mod fuzzy;
mod render;
mod selection;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;

pub use actions::{
    Cancel, Confirm, SecondaryConfirm, SelectFirst, SelectLast, SelectNext, SelectPrevious,
};
pub use delegate::{Direction, PickerDelegate};
pub use render::match_label;

use gpui::{
    App, AppContext as _, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    Pixels, ScrollStrategy, Subscription, Task, UniformListScrollHandle, WeakEntity, Window, px,
};
use oxikube_keymap::{KeyContextual, contexts};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::modal::ModalView;

use fuzzy::QueryGeneration;

/// The picker's width unless [`Picker::width`] says otherwise (before UI zoom).
pub const DEFAULT_WIDTH: Pixels = px(560.);

/// The most the list of matches grows to unless [`Picker::max_height`] says otherwise (before UI
/// zoom); longer lists scroll.
pub const DEFAULT_MAX_HEIGHT: Pixels = px(360.);

/// The height of one row of the list (before UI zoom). Every row has it: the list is a
/// `uniform_list`, which builds only the rows on screen.
pub const ROW_HEIGHT: Pixels = px(28.);

/// A query field over the matches of a [`PickerDelegate`]. See the [module docs](self).
pub struct Picker<D: PickerDelegate> {
    /// The delegate, public as in Zed so the host can read and change its data.
    pub delegate: D,
    query: Entity<InputState>,
    scroll: UniformListScrollHandle,
    /// The newest match update. A newer query replaces (and so cancels) it; it is never cleared
    /// from inside itself.
    pending_update_matches: Option<Task<()>>,
    /// The generation of the newest query.
    generation: QueryGeneration,
    /// The generation whose update has finished.
    settled: u64,
    /// A confirm (`Some(secondary)`) asked for while matches were being updated: it runs when they
    /// arrive.
    confirm_on_update: Option<bool>,
    width: Pixels,
    max_height: Pixels,
    /// Whether [`PickerDelegate::dismissed`] has been called (or is on its way).
    dismissed: bool,
    this: WeakEntity<Self>,
    _query_events: Subscription,
}

impl<D: PickerDelegate> Picker<D> {
    /// A picker over `delegate` with a query field and a `uniform_list` of matches (Zed's
    /// constructor name). Asks the delegate for the matches of the empty query at once.
    pub fn uniform_list(delegate: D, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = delegate.placeholder_text(window, cx);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let query_events = cx.subscribe_in(&query, window, Self::on_query_event);
        let mut picker = Self {
            delegate,
            query,
            scroll: UniformListScrollHandle::new(),
            pending_update_matches: None,
            generation: QueryGeneration::default(),
            settled: 0,
            confirm_on_update: None,
            width: DEFAULT_WIDTH,
            max_height: DEFAULT_MAX_HEIGHT,
            dismissed: false,
            this: cx.weak_entity(),
            _query_events: query_events,
        };
        picker.update_matches(String::new(), window, cx);
        picker
    }

    /// Sets the picker's width (before UI zoom).
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    /// Sets the most the list grows to before it scrolls (before UI zoom).
    pub fn max_height(mut self, max_height: Pixels) -> Self {
        self.max_height = max_height;
        self
    }

    /// Moves the keyboard focus to the query field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.query.update(cx, |query, cx| query.focus(window, cx));
    }

    /// The query field's text.
    pub fn query(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }

    /// Replaces the query (as if typed) and updates the matches.
    pub fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |field, cx| {
            field.set_value(query.to_owned(), window, cx);
        });
        self.update_matches(query.to_owned(), window, cx);
    }

    /// Asks the delegate for the matches of the current query again (the data changed).
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.update_matches(query, window, cx);
    }

    /// Whether a match update is still running (a confirm now waits for it).
    pub fn is_matching(&self) -> bool {
        self.settled != self.generation.current()
    }

    /// Asks the delegate for the matches of `query`. The update of an older query is dropped,
    /// which cancels it.
    pub fn update_matches(&mut self, query: String, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.generation.next();
        let delegate_update = self.delegate.update_matches(query, window, cx);
        // A delegate that matched synchronously is done already: show it in this frame.
        self.matches_updated(cx);
        self.pending_update_matches = Some(cx.spawn_in(window, async move |this, cx| {
            delegate_update.await;
            this.update_in(cx, |this, window, cx| {
                this.finish_update(generation, window, cx);
            })
            .ok();
        }));
    }

    /// The update of `generation` finished: if it is still the newest, reveal the selection and
    /// run the confirm the user asked for meanwhile.
    fn finish_update(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.generation.is_current(generation) {
            return;
        }
        self.settled = generation;
        self.matches_updated(cx);
        if let Some(secondary) = self.confirm_on_update.take() {
            self.delegate.confirm(secondary, window, cx);
        }
    }

    fn matches_updated(&mut self, cx: &mut Context<Self>) {
        if self.delegate.match_count() > 0 {
            self.scroll_to_item_index(self.delegate.selected_index());
        }
        cx.notify();
    }

    fn scroll_to_item_index(&mut self, ix: usize) {
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
    }

    fn on_query_event(
        &mut self,
        query: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Change = event {
            let text = query.read(cx).value().to_string();
            self.update_matches(text, window, cx);
        }
    }

    /// Calls [`PickerDelegate::dismissed`] unless it already ran.
    fn dismiss_delegate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dismissed {
            self.dismissed = true;
            self.delegate.dismissed(window, cx);
        }
    }
}

impl<D: PickerDelegate> EventEmitter<DismissEvent> for Picker<D> {}

impl<D: PickerDelegate> Focusable for Picker<D> {
    /// The query field: focusing the picker puts the caret there.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl<D: PickerDelegate> KeyContextual for Picker<D> {
    const KEY_CONTEXT: &'static str = contexts::PICKER;
}

impl<D: PickerDelegate> ModalView for Picker<D> {
    /// The modal layer closes the picker (a click outside, or the `DismissEvent` after a confirm or
    /// a cancel): tell the delegate once. The call is deferred because the layer holds the picker
    /// while it asks; the strong handle keeps the picker alive until the call has run.
    fn on_before_dismiss(&mut self, window: &mut Window, cx: &mut App) -> bool {
        if !self.dismissed {
            self.dismissed = true;
            if let Some(this) = self.this.upgrade() {
                window.defer(cx, move |window, cx| {
                    this.update(cx, |picker, cx| picker.delegate.dismissed(window, cx));
                });
            }
        }
        true
    }
}
