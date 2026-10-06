//! The catalog's UI-local actions.
//!
//! Actions that stand for a `Command` (connect, disconnect, favourite) are dispatched as the
//! command with the selected cluster as its argument; these actions only name the key or menu
//! entry that does so, or move the selection or the focus (like the table's `SelectNext`).
//! Their default bindings live in the per-OS keymap files of `oxikube_assets`, in the sections
//! for the `Catalog` key context (`oxikube_keymap::contexts::CATALOG`), so users rebind them in
//! `keymap.json` like any other.

use gpui::actions;

actions!(
    catalog,
    [
        /// Move the selection to the next cluster.
        SelectNext,
        /// Move the selection to the previous cluster.
        SelectPrevious,
        /// Move the selection to the first cluster.
        SelectFirst,
        /// Move the selection to the last cluster.
        SelectLast,
        /// Connect the selected cluster (`cluster::Connect`).
        ConnectSelected,
        /// Disconnect the selected cluster (`cluster::Disconnect`).
        DisconnectSelected,
        /// Mark or unmark the selected cluster as a favourite (`cluster::ToggleFavourite`).
        ToggleFavouriteSelected,
        /// Move the focus to the search field.
        FocusSearch,
    ]
);
