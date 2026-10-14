//! The warning badge column's side of the table (E14-S04): whether the table has it, and what
//! makes it redraw (the badges changed, or one expired).

use gpui::{App, Context};

use super::view::ResourceTable;

impl ResourceTable {
    /// Whether the table has the warning badge column (the app has an events index).
    pub(super) fn has_badges(&self, cx: &App) -> bool {
        self.table.read(cx, |d| d.warnings.is_some())
    }

    /// The badges changed (a warning arrived, went, or the window moved): redraw at frame
    /// cadence. A table that is not on screen is not redrawn.
    pub(crate) fn badges_changed(&mut self, cx: &mut Context<Self>) {
        self.redraw.notify(cx);
    }

    /// Whether a badge expired since the last tick (a warning aged past the window).
    pub(super) fn badges_expired(&self, cx: &mut Context<Self>) -> bool {
        let now = self.now();
        self.table.update_quiet(cx, |d| {
            d.warnings
                .as_mut()
                .is_some_and(|warnings| warnings.expired_moved(now))
        })
    }
}
