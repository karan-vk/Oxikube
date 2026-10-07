//! Moving the table's selection on behalf of the detail drawer (E07-U559): `j` / `k` and the
//! arrows in the drawer step to the next or previous row and show that object's detail.

use gpui::Context;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;

use super::view::ResourceTable;

impl ResourceTable {
    /// Steps from the row of `from` (the object the drawer shows) by `delta` rows, selecting the
    /// row it lands on alone (scrolled into view), and dispatches `resource::Open` for it so the
    /// drawer follows. The step is anchored on `from`, not on the table's own cursor, so it is
    /// right when the drawer was reached by an owner link or the palette. Does nothing when
    /// `from` is not listed (filtered out, or another namespace) or the step is clamped to where
    /// it is (the first or last row). Unlike [`Self::open_object`] this opens the object's own
    /// detail on the CRD list too, where the drawer is showing a definition.
    pub fn step_detail(&mut self, from: &ResourceRef, delta: isize, cx: &mut Context<Self>) {
        let landed = self.table.update(cx, |d| {
            let at = d.rows.iter().position(|row| {
                let meta = row.meta();
                *meta.name == *from.name && meta.namespace.as_deref() == from.namespace.as_deref()
            })?;
            let last = d.rows.len().checked_sub(1)?;
            let to = at.saturating_add_signed(delta).min(last);
            if to == at {
                return None;
            }
            d.selection.go_to(&d.rows, to, false);
            Some(to)
        });
        let Some(row) = landed else {
            return;
        };
        self.table.reveal_row(row, cx);
        self.selection_changed(cx);
        if let Some(key) = self.row_key(row, cx) {
            let target = self.resource_ref(key);
            self.deps
                .dispatcher
                .dispatch(Command::ResourceOpen { target }, cx);
        }
    }
}
