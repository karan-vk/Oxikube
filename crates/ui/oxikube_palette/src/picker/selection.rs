// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/picker/src/picker.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! Keyboard selection, confirm, cancel and clicks: Zed's picker keyboard handling.
//!
//! `SelectNext` / `SelectPrevious` wrap around; every move skips the matches the delegate says
//! cannot be selected; a confirm asked for while matches are still being updated waits for them
//! (Zed's `confirm_on_update`). Adapted: `pending_update_matches.is_some()` became
//! [`Picker::is_matching`], and confirming no longer drops the pending update.

use gpui::{ClickEvent, Context, DismissEvent, Window};

use super::{
    Cancel, Confirm, Direction, Picker, PickerDelegate, SecondaryConfirm, SelectFirst, SelectLast,
    SelectNext, SelectPrevious,
};

impl<D: PickerDelegate> Picker<D> {
    /// Handles the selecting an index, and passing the change to the delegate.
    /// If `fallback_direction` is set to `None`, the index will not be selected
    /// if the element at that index cannot be selected.
    /// If `fallback_direction` is set to
    /// `Some(..)`, the next selectable element will be selected in the
    /// specified direction (Down or Up), cycling through all elements until
    /// finding one that can be selected or returning if there are no selectable elements.
    /// If `scroll_to_index` is true, the new selected index will be scrolled into
    /// view.
    pub fn set_selected_index(
        &mut self,
        mut ix: usize,
        fallback_direction: Option<Direction>,
        scroll_to_index: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let match_count = self.delegate.match_count();
        if match_count == 0 || ix >= match_count {
            return;
        }

        if let Some(bias) = fallback_direction {
            let mut curr_ix = ix;
            while !self.delegate.can_select(curr_ix, window, cx) {
                curr_ix = match bias {
                    Direction::Down => {
                        if curr_ix == match_count - 1 {
                            0
                        } else {
                            curr_ix + 1
                        }
                    }
                    Direction::Up => {
                        if curr_ix == 0 {
                            match_count - 1
                        } else {
                            curr_ix - 1
                        }
                    }
                };
                // There is no item that can be selected
                if ix == curr_ix {
                    return;
                }
            }
            ix = curr_ix;
        } else if !self.delegate.can_select(ix, window, cx) {
            return;
        }

        let previous_index = self.delegate.selected_index();
        self.delegate.set_selected_index(ix, window, cx);
        let current_index = self.delegate.selected_index();

        if previous_index != current_index && scroll_to_index {
            self.scroll_to_item_index(ix);
        }
        cx.notify();
    }

    /// Selects the next match, wrapping to the first.
    pub fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.match_count();
        if count > 0 {
            let index = self.delegate.selected_index();
            let ix = if index >= count - 1 { 0 } else { index + 1 };
            self.set_selected_index(ix, Some(Direction::Down), true, window, cx);
        }
    }

    /// Selects the previous match, wrapping to the last.
    pub fn select_previous(
        &mut self,
        _: &SelectPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.delegate.match_count();
        if count > 0 {
            let index = self.delegate.selected_index().min(count - 1);
            let ix = if index == 0 { count - 1 } else { index - 1 };
            self.set_selected_index(ix, Some(Direction::Up), true, window, cx);
        }
    }

    /// Selects the first selectable match.
    pub fn select_first(&mut self, _: &SelectFirst, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.match_count();
        if count > 0 {
            self.set_selected_index(0, Some(Direction::Down), true, window, cx);
        }
    }

    /// Selects the last selectable match.
    pub fn select_last(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.match_count();
        if count > 0 {
            self.set_selected_index(count - 1, Some(Direction::Up), true, window, cx);
        }
    }

    /// Closes the picker without confirming: the delegate hears [`PickerDelegate::dismissed`], and
    /// the modal layer closes the picker and gives the focus back.
    pub fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_delegate(window, cx);
        cx.emit(DismissEvent);
    }

    /// Confirms the selected match, or, while matches are being updated, once they arrive.
    pub fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_or_wait(false, window, cx);
    }

    /// [`Self::confirm`] with `secondary` set.
    pub fn secondary_confirm(
        &mut self,
        _: &SecondaryConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_or_wait(true, window, cx);
    }

    fn confirm_or_wait(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_matching() {
            self.confirm_on_update = Some(secondary);
        } else {
            self.delegate.confirm(secondary, window, cx);
        }
    }

    /// A click on row `ix`: select it and confirm (secondary with the platform modifier held).
    pub(super) fn handle_click(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        window.prevent_default();
        if ix >= self.delegate.match_count() || !self.delegate.can_select(ix, window, cx) {
            return;
        }
        self.set_selected_index(ix, None, false, window, cx);
        self.delegate
            .confirm(event.modifiers().secondary(), window, cx);
    }
}
