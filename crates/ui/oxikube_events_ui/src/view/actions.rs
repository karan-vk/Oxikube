//! The events view's key actions, bound in the `EventsView` key context.
//!
//! The defaults are in the per-OS keymap files of `oxikube_assets`: k9s' `ctrl-z` shows only the
//! faults, `p` pauses the live tail, `c` (and `cmd-c` / `ctrl-c`) copies the selected row,
//! `shift-c` copies it as JSON, `ctrl-s` exports the listed events, `enter` opens the event's
//! object, the arrows (and `j` / `k`, `g` / `shift-g`, `home` / `end`) move the selection, `/`
//! goes to the text filter and `escape` back to the rows. Each action that stands for a command
//! dispatches it with the view's cluster (`events::ToggleFaults`, ..., `resource::Open`), so the
//! key, the toolbar, the palette and an agent run one behaviour. Moving the selection and the
//! focus is the view's own business. Users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    events_view,
    [
        /// Show only the faults (`Warning` events), or every event (`ctrl-z`,
        /// `events::ToggleFaults`).
        ToggleFaults,
        /// Pause the live tail, or resume it (`p`, `events::TogglePause`).
        TogglePause,
        /// Copy the selected row as text (`c`, `events::CopyRow`).
        CopyRow,
        /// Copy the selected event as JSON (`shift-c`, `events::CopyRowJson`).
        CopyRowJson,
        /// Write the listed events to a JSON file (`ctrl-s`, `events::ExportJson`).
        ExportJson,
        /// Open the selected event's object (`enter`, `resource::Open`).
        OpenSelected,
        /// Select the next row (`down`, `j`).
        SelectNext,
        /// Select the previous row (`up`, `k`).
        SelectPrevious,
        /// Select the first row (`home`, `g`).
        SelectFirst,
        /// Select the last row (`end`, `shift-g`).
        SelectLast,
        /// Put the focus in the text filter (`/`).
        FocusFilter,
        /// Give the focus back to the rows (`escape` in a filter field).
        FocusTable,
    ]
);
