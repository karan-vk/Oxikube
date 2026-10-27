//! The page's key actions, bound in the `RbacSubjects` key context.
//!
//! The defaults are in the per-OS keymap files of `oxikube_assets`: the arrows (and `j` / `k`,
//! `g` / `shift-g`, `home` / `end`) move the selection, `enter` opens the selected subject's
//! matrix, `/` goes to the filter, `h` / `l` pick the verb column whose granting bindings the
//! detail lists, `i` includes (or leaves out) the groups the API server adds on its own, `s`
//! cycles the scope (cluster-wide, then each namespace), `y` copies the matrix as text and
//! `escape` goes back to the list. Copying is a command (`rbac::CopyMatrix`), so the key, the
//! button, the palette and an agent run one behaviour; the rest only move around inside the page.
//! Users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    rbac_subjects,
    [
        /// Select the next row (`down`, `j`).
        SelectNext,
        /// Select the previous row (`up`, `k`).
        SelectPrevious,
        /// Select the first row (`home`, `g`).
        SelectFirst,
        /// Select the last row (`end`, `shift-g`).
        SelectLast,
        /// Put the focus in the filter (`/`).
        FocusFilter,
        /// Give the focus back to the rows (`escape` in the filter).
        FocusTable,
        /// Open the matrix of the selected subject (`enter`).
        OpenSelected,
        /// Go back from a matrix to the list it came from (`escape`).
        Back,
        /// The verb column to the right (`l`).
        NextColumn,
        /// The verb column to the left (`h`).
        PreviousColumn,
        /// Include or leave out the implicit groups (`i`).
        ToggleImplicitGroups,
        /// The next scope: cluster-wide, then each namespace (`s`).
        NextScope,
        /// Copy the matrix as tab-separated text (`y`, `rbac::CopyMatrix`).
        CopyMatrix,
    ]
);
