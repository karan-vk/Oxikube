//! The picker's actions, under the names of Zed's `menu` crate.
//!
//! The picker binds no keystroke itself: the default bindings live in the per-OS keymap files of
//! `oxikube_assets`, in the sections for the `Picker` key context
//! ([`oxikube_keymap::contexts::PICKER`]) and `Picker > Input` (the query field, whose own
//! bindings for the arrows, Enter and Escape would otherwise win), so users rebind them in
//! `keymap.json` like any other.

use gpui::actions;

actions!(
    picker,
    [
        /// Select the next match (wraps to the first).
        SelectNext,
        /// Select the previous match (wraps to the last).
        SelectPrevious,
        /// Select the first match.
        SelectFirst,
        /// Select the last match.
        SelectLast,
        /// Confirm the selected match.
        Confirm,
        /// Confirm the selected match the alternative way the delegate defines.
        SecondaryConfirm,
        /// Close the picker without confirming.
        Cancel,
    ]
);
