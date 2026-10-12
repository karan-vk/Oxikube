//! The page's key actions, bound in the `Pulses` key context.
//!
//! The defaults are in the per-OS keymap files of `oxikube_assets`: the arrow keys and k9s' `hjkl`
//! move between tiles, `home` / `end` jump to the first and last, `enter` opens the selected
//! tile's table, `r` counts again the kinds that have no feed, and `escape` closes the page.
//! Users rebind them in `keymap.json`.
//!
//! Moving the selection, counting again and closing the tab are the page's own business, like a
//! cursor key in a table. `Open` is the one that does something: it sends `resource::OpenList`
//! on the bus, the command the sidebar, the palette and the `:` jump bar send for the same table.

use gpui::actions;

actions!(
    pulses,
    [
        /// Select the tile to the left (`h`, `left`).
        SelectLeft,
        /// Select the tile to the right (`l`, `right`).
        SelectRight,
        /// Select the tile above (`k`, `up`).
        SelectUp,
        /// Select the tile below (`j`, `down`).
        SelectDown,
        /// Select the first tile (`home`).
        SelectFirst,
        /// Select the last tile (`end`).
        SelectLast,
        /// Open the table of the selected tile's kind (`enter`, `resource::OpenList`).
        Open,
        /// Count again the kinds that have no feed (`r`).
        Refresh,
        /// Close the page and go back to the tab before it (`escape`).
        Close,
    ]
);
