//! The log view's key actions, bound in the `LogView` key context.
//!
//! The defaults are k9s's, in the per-OS keymap files of `oxikube_assets`: `0` tail, `1` head,
//! `2`-`6` since 1m / 5m / 15m / 30m / 1h, `s` autoscroll, `w` wrap, `t` timestamps, `p` previous
//! container, `f` fullscreen, `m` mark, `c` copy. Each action that stands for a command
//! dispatches it with the view's target (`logs::SetRange`, `logs::ToggleWrap`, ...), so the key,
//! the toolbar, the palette and an agent run one behaviour (non-negotiable 4). Users rebind them
//! in `keymap.json`.
//!
//! `m` marks the focused line, `c` copies the selection (else the lines on screen), `shift-c`
//! clears the buffer, `ctrl-s` saves the whole buffer and `ctrl-shift-s` the lines on screen to a
//! file, `escape` drops the selection. Mark, copy, clear and the two saves dispatch their `logs::*`
//! command like the rest; `escape` only changes the selection, which is a view's own business.

use gpui::actions;

actions!(
    log_view,
    [
        /// Read the newest lines and follow (`0`, `logs::SetRange` tail).
        Tail,
        /// Read the log from its start (`1`, `logs::SetRange` head).
        Head,
        /// Read the last minute and follow (`2`).
        Since1m,
        /// Read the last 5 minutes and follow (`3`).
        Since5m,
        /// Read the last 15 minutes and follow (`4`).
        Since15m,
        /// Read the last 30 minutes and follow (`5`).
        Since30m,
        /// Read the last hour and follow (`6`).
        Since1h,
        /// Follow the newest line, or stop following (`s`, `logs::ToggleAutoscroll`).
        ToggleAutoscroll,
        /// Wrap long lines, or not (`w`, `logs::ToggleWrap`).
        ToggleWrap,
        /// Show the timestamps, or not (`t`, `logs::ToggleTimestamps`).
        ToggleTimestamps,
        /// Read the previous container instance, or the current one (`p`, `logs::TogglePrevious`).
        TogglePrevious,
        /// Fill the cluster tab, or not (`f`, `logs::ToggleFullscreen`).
        ToggleFullscreen,
        /// Mark the focused line, or unmark it (`m`, `logs::Mark`).
        Mark,
        /// Copy the selected lines, else the lines on screen (`c`, `logs::Copy`).
        Copy,
        /// Empty the local buffer and the view; the stream goes on (`shift-c`, `logs::Clear`).
        Clear,
        /// Save everything the buffer holds to a file (`ctrl-s`, `logs::Save` with scope `all`).
        SaveAll,
        /// Save the lines on screen to a file (`ctrl-shift-s`, `logs::Save` with scope `visible`).
        SaveVisible,
        /// Select nothing (`escape`).
        ClearSelection,
    ]
);
