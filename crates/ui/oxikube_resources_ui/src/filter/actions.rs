//! The filter bar's key actions.
//!
//! `/` ([`FocusFilter`]) is a view action that stands for the `table::FocusFilter` command: the
//! table focuses the bar at once (so the keys typed right after `/` are text) and dispatches the
//! command; when the bus hands it back to [`ResourceViews`](crate::ResourceViews) it is only an
//! echo and does nothing. From the palette or an agent the command focuses the bar. `escape`
//! while typing ([`ClearFilter`]) clears the filter and returns focus to the table; `enter`
//! returns focus without clearing (the bar handles it on the input's `PressEnter`). The default
//! bindings are in the keymap files of `oxikube_assets`: `/` in `Table && !Editing`, `escape` in
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
