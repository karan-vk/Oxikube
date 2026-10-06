//! The results of a finished delete: one line per object.

use gpui::{IntoElement, ParentElement as _, Styled as _, div, px};
use oxikube_app::{DeleteReport, ItemStatus};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Colors, Icon, IconName, u};

/// The height of one result row, at 100 % zoom: uniform, as `uniform_list` needs.
pub(super) const ROW_HEIGHT: f32 = 24.0;

/// "Pod default/web-0", "Namespace payments".
pub(super) fn object_label(target: &oxikube_domain::ids::ResourceRef) -> String {
    match target.namespace() {
        Some(ns) => format!("{} {ns}/{}", target.gvk.kind, target.name),
        None => format!("{} {}", target.gvk.kind, target.name),
    }
}

/// Row `ix` of `report`'s results.
pub(super) fn result_row(report: &DeleteReport, ix: usize, colors: &Colors) -> impl IntoElement {
    let item = report.items.get(ix);
    let (icon, tone) = match item.map(|i| &i.status) {
        Some(ItemStatus::Deleted | ItemStatus::Deleting) => (IconName::CircleCheck, colors.success),
        Some(ItemStatus::Forbidden(_)) => (IconName::TriangleAlert, colors.warning),
        Some(ItemStatus::NotFound(_)) => (IconName::CircleAlert, colors.warning),
        Some(ItemStatus::Failed(_)) | None => (IconName::CircleX, colors.error),
    };
    let label = item.map(|i| object_label(&i.target)).unwrap_or_default();
    let status = item.map(|i| i.status.label()).unwrap_or_default();
    let detail = item
        .and_then(|i| i.status.message().map(str::to_owned))
        .unwrap_or_default();
    h_flex()
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
                .child(detail),
        )
}
