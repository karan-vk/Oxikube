// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/picker/src/picker.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! [`PickerDelegate`]: what a picker shows and what confirming does.
//!
//! The method set and order are Zed's (`match_count`, `selected_index`, `set_selected_index`,
//! `can_select`, `placeholder_text`, `no_matches_text`, `update_matches`, `confirm`, `dismissed`,
//! `render_match`, `render_header`, `render_footer`), so a diff against upstream stays readable.
//! Left out: persistence (`name`), previews, multi-select, history, completion, hover selection
//! and the editor overrides; none of Oxikube's pickers need them yet.

use gpui::{AnyElement, App, Context, IntoElement, SharedString, Task, Window};

use super::Picker;

/// The direction keyboard selection moves in, for skipping matches that cannot be selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Towards the first match.
    Up,
    /// Towards the last match.
    Down,
}

/// Owns a picker's matches: filters them for a query, renders one, and acts on confirm.
///
/// The picker owns the query field, the virtual list, keyboard handling and dismissal; the
/// delegate owns the data. Indexes are positions in the current matches (`0..match_count()`).
pub trait PickerDelegate: Sized + 'static {
    /// The element one match renders as (inside the picker's row: the row draws the selection
    /// background and handles clicks).
    type ListItem: IntoElement;

    /// How many matches the current query has.
    fn match_count(&self) -> usize;

    /// The selected match. Meaningless while [`Self::match_count`] is zero.
    fn selected_index(&self) -> usize;

    /// Selects match `ix` (always `< match_count()` and accepted by [`Self::can_select`]).
    fn set_selected_index(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    );

    /// Whether match `ix` can be selected; keyboard selection skips the ones that cannot (a
    /// section header, say) and clicks on them do nothing. Default: every match can.
    fn can_select(
        &self,
        _ix: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> bool {
        true
    }

    /// What the empty query field says (Zed returns an `Arc<str>`; the query field here takes a
    /// [`SharedString`]).
    fn placeholder_text(&self, window: &mut Window, cx: &mut App) -> SharedString;

    /// What the picker shows when the query matches nothing; `None` shows nothing.
    fn no_matches_text(&self, _window: &mut Window, _cx: &mut App) -> Option<SharedString> {
        Some("No matches".into())
    }

    /// Recomputes the matches for `query`. The picker calls it on every edit of the query and
    /// awaits the returned task before it reveals the selection again (and runs a confirm the user
    /// asked for meanwhile).
    ///
    /// Filter large candidate sets off the UI thread (`cx.background_spawn`, or
    /// [`super::fuzzy::match_strings_async`]) and write the result inside the returned task. A
    /// newer query drops the task of the older one, which cancels it; a delegate whose work can
    /// outlive that (a detached task) must compare a [`super::fuzzy::QueryGeneration`] before it
    /// writes, so a slow old query never overwrites a newer one.
    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()>;

    /// Acts on the selected match: Enter (`secondary == false`), or the secondary confirm key or a
    /// platform-modified click (`secondary == true`). Emit `DismissEvent` (`cx.emit(DismissEvent)`)
    /// to close the picker afterwards.
    fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>);

    /// The picker is closing: Escape, a click outside it, or a `DismissEvent` (after a confirm
    /// too). Called once per picker. Not called when another modal replaces it.
    fn dismissed(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>);

    /// Renders match `ix`; `selected` says whether it is the selected one. Every match must have
    /// the same height: the list is a `uniform_list` and measures one row for all of them.
    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem>;

    /// An element between the query field and the matches (a title, a hint). Default: none.
    fn render_header(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<AnyElement> {
        None
    }

    /// An element under the matches (key hints, actions). Default: none.
    fn render_footer(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<AnyElement> {
        None
    }
}
