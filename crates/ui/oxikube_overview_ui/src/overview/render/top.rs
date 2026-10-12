//! The top consumers: pods and namespaces by CPU and memory, from the last metrics-server poll.
//! A row's click opens the pod or the namespace.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, u};

use super::{card, muted};
use crate::overview::model::TopRow;
use crate::overview::view::ClusterOverview;

impl ClusterOverview {
    pub(super) fn top(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let top = &self.shown.top;
        let lists: [(&'static str, &'static str, &[TopRow]); 4] = [
            ("overview-top-pods-cpu", "Pods by CPU", &top.pods_cpu),
            (
                "overview-top-pods-memory",
                "Pods by memory",
                &top.pods_memory,
            ),
            (
                "overview-top-namespaces-cpu",
                "Namespaces by CPU",
                &top.namespaces_cpu,
            ),
            (
                "overview-top-namespaces-memory",
                "Namespaces by memory",
                &top.namespaces_memory,
            ),
        ];
        let empty = if self.shown.notice.is_some() {
            "–"
        } else {
            "No readings yet"
        };
        let columns = lists.into_iter().map(|(id, title, rows)| {
            let mut list = v_flex().flex_1().min_w(u(px(160.))).gap(u(px(2.))).child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(title),
            );
            if rows.is_empty() {
                list = list.child(muted(empty, id, cx));
            }
            for (ix, row) in rows.iter().enumerate() {
                list = list.child(self.top_row(id, ix, row, cx));
            }
            list
        });
        let columns: Vec<_> = columns.collect();
        let scope = self
            .top_scope
            .clone()
            .map(|note| muted(note, "overview-top-scope", cx));
        card("overview-top", "Top consumers", cx)
            .children(scope)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(u(tokens.spacing.lg))
                    .children(columns),
            )
            .into_any_element()
    }

    fn top_row(
        &self,
        list: &'static str,
        ix: usize,
        row: &TopRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let (namespace, name) = row.target.clone();
        h_flex()
            .id((list, ix))
            .debug_selector(move || format!("{list}-{ix}"))
            .gap(u(tokens.spacing.md))
            .px(u(px(4.)))
            .rounded(u(tokens.radius.sm))
            .cursor_pointer()
            .hover(|s| s.bg(tokens.colors.element_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_row(namespace.as_deref(), &name, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(row.label.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(tokens.colors.text_muted)
                    .child(SharedString::from(row.value.clone())),
            )
            .into_any_element()
    }
}
