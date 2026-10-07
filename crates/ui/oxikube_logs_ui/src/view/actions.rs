//! The log view's key actions, bound in the `LogView` key context.
//!
//! The defaults are k9s's, in the per-OS keymap files of `oxikube_assets`: `0` tail, `1` head,
//! `2`-`6` since 1m / 5m / 15m / 30m / 1h, `s` autoscroll, `w` wrap, `t` timestamps, `p` previous
//! container, `f` fullscreen, `m` mark, `c` copy. Each action that stands for a command
//! dispatches it with the view's target (`logs::SetRange`, `logs::ToggleWrap`, ...), so the key,
//! the toolbar, the palette and an agent run one behaviour (non-negotiable 4). Users rebind them
//! in `keymap.json`.
//!
//! The search keys (E08-S03): `/` or `cmd-f` open the bar (`Find`), `enter` / `shift-enter` in it,
//! or `n` / `N` outside it, step through the matches, `escape` closes it, `alt-c` / `alt-i` /
//! `alt-f` toggle case, inverse and filter mode. Each dispatches its `logs::*` command too.
//!
//! `Mark` and `Copy` are bound here so their keys are reserved; what they do arrives with
//! E08-S06 (export, copy, mark, clear).

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
        /// Mark the current position (`m`; E08-S06).
        Mark,
        /// Copy the lines (`c`; E08-S06).
        Copy,
        /// Open the search bar (`/`, `cmd-f`, `logs::Find`).
        Find,
        /// Go to the next match (`enter` in the bar, `n`, `logs::NextMatch`).
        NextMatch,
        /// Go to the previous match (`shift-enter` in the bar, `shift-n`, `logs::PreviousMatch`).
        PreviousMatch,
        /// Make the search case-sensitive, or not (`alt-c`, `logs::ToggleCase`).
        ToggleCase,
        /// Match the lines without the pattern, or those with it (`alt-i`, `logs::ToggleInverse`).
        ToggleInverse,
        /// Show only the matching lines, or all with the matches highlighted (`alt-f`,
        /// `logs::ToggleFilterMode`).
        ToggleFilterMode,
        /// Close the search bar and clear the search (`escape`, `logs::CloseSearch`).
        CloseSearch,
    ]
);
