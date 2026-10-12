//! One result row of the bulk dialog's virtualised list.

use gpui::{InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px};
use oxikube_app::{BulkReport, BulkStatus, object_label};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Colors, Icon, IconName, u};

/// The height of one result row, at 100 % zoom: uniform, as `uniform_list` needs.
pub(super) const ROW_HEIGHT: f32 = 24.0;

/// Row `ix` of `report`: an icon, the object, the status word and the server's message.
pub(super) fn result_row(report: &BulkReport, ix: usize, colors: &Colors) -> impl IntoElement {
    let row = report.rows.get(ix);
    let (icon, tone) = match row.map(|row| &row.status) {
        Some(BulkStatus::Ok(_)) => (IconName::CircleCheck, colors.success),
        Some(BulkStatus::Skipped(_)) => (IconName::CircleAlert, colors.warning),
        Some(BulkStatus::Failed(_)) | None => (IconName::CircleX, colors.error),
    };
    let label = row.map(|row| object_label(&row.target)).unwrap_or_default();
    let status = row.map(|row| row.status.label()).unwrap_or_default();
    let message = row
        .map(|row| row.status.message().to_owned())
        .unwrap_or_default();
    h_flex()
        .debug_selector(move || format!("bulk-result-{ix}"))
        .h(u(px(ROW_HEIGHT)))
        .w_full()
        .gap(u(px(8.)))
        .items_center()
        .child(Icon::new(icon).size(u(px(14.))).color(tone))
        .child(div().flex_none().text_color(colors.text).child(label))
        .child(div().flex_none().text_color(tone).child(status))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(colors.text_muted)
                .child(message),
        )
}
