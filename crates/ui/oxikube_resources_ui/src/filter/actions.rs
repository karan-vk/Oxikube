//! The filter bar's key actions.
//!
//! `/` ([`FocusFilter`]) is a view action that stands for the `table::FocusFilter` command: the
//! table dispatches the command, the bus hands it back to
//! [`ResourceViews`](crate::ResourceViews), which focuses the bar. `escape` while typing
//! ([`ClearFilter`]) clears the filter and returns focus to the table; `enter` returns focus
//! without clearing (the bar handles it on the input's `PressEnter`). The default bindings are in
//! the keymap files of `oxikube_assets`: `/` in `Table && !Editing`, `escape` in
//! `Table && Editing`.

use gpui::actions;

actions!(
    resource_table,
    [
        /// Focus the filter bar (`table::FocusFilter`).
        FocusFilter,
        /// Clear the filter and return focus to the table.
        ClearFilter,
    ]
);
