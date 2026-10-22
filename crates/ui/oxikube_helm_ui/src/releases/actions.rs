//! The page's key actions, bound in the `HelmReleases` key context.
//!
//! The defaults are in the per-OS keymap files of `oxikube_assets`: the arrows (and `j` / `k`,
//! `g` / `shift-g`, `home` / `end`) move the selection, which opens the release's detail,
//! `/` goes to the filter, `[` and `]` switch the detail's tab, `v` flips the Values tab between
//! the user's values and all of them, `x` reveals (or hides again) the masked values, and
//! `escape` closes the detail (or, in the filter, gives the rows the focus back). Revealing is a
//! command (`helm::RevealValues`), so the key, the button and an agent run one behaviour; the
//! rest only move around inside the page. Users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    helm_releases,
    [
        /// Select the next release (`down`, `j`).
        SelectNext,
        /// Select the previous release (`up`, `k`).
        SelectPrevious,
        /// Select the first release (`home`, `g`).
        SelectFirst,
        /// Select the last release (`end`, `shift-g`).
        SelectLast,
        /// Put the focus in the filter (`/`).
        FocusFilter,
        /// Give the focus back to the rows (`escape` in the filter).
        FocusTable,
        /// Show the next detail tab (`]`).
        NextTab,
        /// Show the previous detail tab (`[`).
        PreviousTab,
        /// Flip the Values tab between the user's values and all of them (`v`).
        ToggleValuesMode,
        /// Reveal the masked values of the selected release, or hide them again (`x`,
        /// `helm::RevealValues`).
        RevealValues,
        /// Close the detail (`escape`).
        CloseDetail,
    ]
);
