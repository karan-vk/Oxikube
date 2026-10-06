//! The resource table's key actions.
//!
//! Moving the cursor and the selection is local to the view (like the catalog's `SelectNext`);
//! the actions that stand for a command dispatch it with the table's rows as its argument:
//! `OpenSelected` sends `resource::Open`, `CopyName` sends `resource::CopyName`, `SelectAll`
//! sends `resource::SelectAll`, so the palette, the context menu, the key and an agent run one
//! behaviour (non-negotiable 4).
//!
//! The default bindings live in the per-OS keymap files of `oxikube_assets`, in the sections
//! for the `Table` key context (`oxikube_keymap::contexts::TABLE`): `j` / `k` and the arrows
//! move, shift extends, `enter` opens, `escape` clears, `cmd-a` / `ctrl-a` selects all,
//! `cmd-c` / `ctrl-c` copies the name, `delete` (and k9s's `ctrl-d`) opens the delete dialog. Users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    resource_table,
    [
        /// Move the cursor to the next row and select it.
        SelectNext,
        /// Move the cursor to the previous row and select it.
        SelectPrevious,
        /// Move the cursor to the first row.
        SelectFirst,
        /// Move the cursor to the last row.
        SelectLast,
        /// Move the cursor one page down.
        SelectPageDown,
        /// Move the cursor one page up.
        SelectPageUp,
        /// Extend the selection to the next row.
        ExtendNext,
        /// Extend the selection to the previous row.
        ExtendPrevious,
        /// Open the cursor row's detail (`resource::Open`).
        OpenSelected,
        /// Copy the cursor row's name (`resource::CopyName`).
        CopyName,
        /// Select every row (`resource::SelectAll`).
        SelectAll,
        /// Clear the selection.
        ClearSelection,
        /// Delete the selected rows, or the cursor row (`resource::Delete` through the delete
        /// dialog).
        DeleteSelected,
    ]
);
