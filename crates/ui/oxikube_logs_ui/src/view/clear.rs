//! Clear (k9s `shift-c`, `logs::Clear`): empties the local buffer and the view; the stream keeps
//! reading and the cluster's logs are untouched.

use gpui::{AppContext as _, Context, ListOffset, Window, point, px};
use oxikube_runtime::notify_coalesced;
use oxikube_workspace::DialogModal;

use super::LogView;
use super::text::group;

impl LogView {
    /// Clears the view, after asking when there are marked lines (they go with the buffer).
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.as_ref().and_then(|w| w.upgrade());
        let marks = self.marks.len();
        let Some(workspace) = workspace.filter(|_| marks > 0) else {
            self.clear_now(cx);
            return;
        };
        let view = cx.entity().downgrade();
        let message = if marks == 1 {
            "1 marked line will go with the buffer. The cluster's logs are not touched and the \
             stream keeps reading."
                .to_owned()
        } else {
            format!(
                "{} marked lines will go with the buffer. The cluster's logs are not touched and \
                 the stream keeps reading.",
                group(marks as u64)
            )
        };
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| {
                DialogModal::new("Clear the log?", cx)
                    .message(message)
                    .confirm_label("Clear")
                    .destructive()
                    .on_confirm(move |_, cx| {
                        view.update(cx, |view, cx| view.clear_now(cx)).ok();
                    })
            });
            workspace.show_modal(dialog, window, cx);
        });
    }

    /// Empties the session's buffer and the view without asking: lines, selection and marks go,
    /// streaming continues, and the view starts at the top of what arrives next. Nothing is
    /// "dropped": no truncated marker appears.
    pub fn clear_now(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let next = session.clear();
        let change = self.window.clear(next);
        self.rows_changed(change);
        self.selection.clear();
        self.marks.clear();
        self.follow.clear_to(next);
        if self.options.wrap {
            self.list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        } else {
            self.scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.), px(0.)));
        }
        self.follow_tail();
        notify_coalesced(cx);
    }
}
