//! Moving the table's selection on behalf of the detail drawer (E07-U559): `j` / `k` and the
//! arrows in the drawer step to the next or previous row and show that object's detail.

use gpui::Context;
use oxikube_domain::command::Command;

use super::view::ResourceTable;

impl ResourceTable {
    /// Moves the cursor by `delta` rows (selecting that row alone, scrolled into view) and
    /// dispatches `resource::Open` for the object it lands on, so the drawer follows. Does nothing
    /// when the cursor cannot move (the first or last row, an empty table). Unlike
    /// [`Self::open_object`] this opens the object's own detail on the CRD list too, where the
    /// drawer is showing a definition.
    pub fn step_detail(&mut self, delta: isize, cx: &mut Context<Self>) {
        let before = self.cursor_row(cx);
        self.move_cursor(delta, false, cx);
        let after = self.cursor_row(cx);
        if after.is_none() || after == before {
            return;
        }
        if let Some(key) = after.and_then(|row| self.row_key(row, cx)) {
            let target = self.resource_ref(key);
            self.deps
                .dispatcher
                .dispatch(Command::ResourceOpen { target }, cx);
        }
    }
}
