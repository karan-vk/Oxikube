//! Keeping a table's badge column current: redraw when the badges change.

use futures::StreamExt as _;
use gpui::{Context, Task};

use crate::table::ResourceTable;

/// The task that redraws `table` (through its render gate: a table that is not on screen is not
/// drawn) whenever the badges change. Coalesced: a burst of events is one wake.
pub(crate) fn follow_badges(
    mut changes: oxikube_app::BadgeChanges,
    cx: &mut Context<ResourceTable>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while changes.next().await.is_some() {
            if this
                .update(cx, |table, cx| table.badges_changed(cx))
                .is_err()
            {
                break;
            }
        }
    })
}
