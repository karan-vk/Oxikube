//! The panel's key actions, bound in the `ApplyResults` key context.
//!
//! The defaults are in the per-OS keymap files of `oxikube_assets`: the arrows (and `j` / `k`,
//! `g` / `shift-g`, `home` / `end`) move the cursor over the rows, `enter` opens or closes a failed
//! row's full message, `e` shows only the failed rows and `o` opens the row's document in the
//! editor. They move around inside the panel; everything that changes a cluster is the buttons'
//! (and the palette's) business. Users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    apply_results,
    [
        /// Select the next row (`down`, `j`).
        SelectNext,
        /// Select the previous row (`up`, `k`).
        SelectPrevious,
        /// Select the first row (`home`, `g`).
        SelectFirst,
        /// Select the last row (`end`, `shift-g`).
        SelectLast,
        /// Open or close the selected row's full message and field paths (`enter`).
        ToggleExpand,
        /// Show only the failed and skipped rows, or all of them (`e`).
        ToggleErrorsOnly,
        /// Open the selected row's document in the editor, at its source line (`o`).
        OpenInEditor,
    ]
);
