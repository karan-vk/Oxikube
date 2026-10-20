//! The panel's key actions, bound in the `PortForwards` key context.
//!
//! The defaults are in the per-OS keymap files of `oxikube_assets`: the arrows (and `j` / `k`,
//! `g` / `shift-g`, `home` / `end`) move the selection, `s` stops the selected forward, `r`
//! restarts it (or starts a stopped one), `d` removes it, `c` copies its URL, `o` or `enter` opens
//! it in the browser, `/` goes to the filter and `escape` closes the panel (or, in the filter,
//! gives the rows the focus back). Each action that stands for a command dispatches it with the
//! selected forward's id, so the key, the toolbar, the palette and an agent run one behaviour.
//! Users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    port_forwards,
    [
        /// Select the next forward (`down`, `j`).
        SelectNext,
        /// Select the previous forward (`up`, `k`).
        SelectPrevious,
        /// Select the first forward (`home`, `g`).
        SelectFirst,
        /// Select the last forward (`end`, `shift-g`).
        SelectLast,
        /// Stop the selected forward (`s`, `portforward::Stop`).
        StopSelected,
        /// Restart the selected forward, or start it when it is stopped (`r`,
        /// `portforward::Restart`).
        RestartSelected,
        /// Remove the selected forward (`d`, `portforward::Remove`).
        RemoveSelected,
        /// Copy the selected forward's URL (`c`, `portforward::CopyUrl`).
        CopyUrlSelected,
        /// Open the selected forward in the browser (`o`, `enter`, `portforward::OpenInBrowser`).
        OpenSelected,
        /// Put the focus in the filter (`/`).
        FocusFilter,
        /// Give the focus back to the rows (`escape` in the filter).
        FocusTable,
        /// Close the panel (`escape`).
        Close,
    ]
);

actions!(
    portforward,
    [
        /// Show and focus the Port Forwards panel, or hide it (`portforward::TogglePanel`).
        TogglePanel,
    ]
);
