//! Drawing the view: the toolbar (filters, the "N new" pill, export), the table or the state that
//! replaces it (loading, unavailable, empty, no match), the detail panel and the status line.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oxikube_domain::audit::AuditExportFormat;
use oxikube_domain::command::Command;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::menu::DropdownMenu as _;
use oxikube_ui::table::Table;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::filters::menu_row_for_export;
use super::{AuditView, Phase, tagged};

/// `4812` as `4,812`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl AuditView {
    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let filtered = !self.filters.is_empty();
        let pill = self.pending_new.filter(|news| !news.is_none()).map(|news| {
            let label = format!("{} new", news.label());
            div()
                .id("audit-new")
                .debug_selector(|| "audit-new".into())
                .flex_none()
                .px(u(px(6.)))
                .rounded(u(px(8.)))
                .bg(tokens.colors.element)
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text)
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| view.show_new(cx)))
                .child(SharedString::from(label))
        });
        let view = cx.entity().downgrade();
        let export = Button::new("audit-export")
            .label("Export")
            .ghost()
            .xsmall()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for format in AuditExportFormat::ALL {
                    let view = view.clone();
                    menu = menu.item(menu_row_for_export(format).on_click(move |_, _, cx| {
                        // The same command the palette sends, so the bus sees every export.
                        view.update(cx, |view, cx| {
                            view.deps
                                .dispatcher
                                .dispatch(Command::AuditExport { format }, cx);
                        })
                        .ok();
                    }));
                }
                menu
            });
        h_flex()
            .id("audit-toolbar")
            .debug_selector(|| "audit-toolbar".into())
            .flex_none()
            .flex_wrap()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .children(self.filter_dropdowns(cx))
            .children(filtered.then(|| {
                tagged(
                    "audit-clear",
                    Button::new("audit-clear")
                        .label("Clear")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|view, _, _, cx| view.clear_filters(cx))),
                )
            }))
            .child(div().flex_1())
            .children(pill)
            .child(div().debug_selector(|| "audit-export".into()).child(export))
            .into_any_element()
    }

    /// What replaces the table while there is nothing to list.
    fn state(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tokens = cx.tokens();
        let empty = self.model(cx).is_empty();
        let (selector, message, action): (&'static str, SharedString, Option<AnyElement>) =
            match (&self.phase, &self.error, empty) {
                (_, Some(message), true) => (
                    "audit-unavailable",
                    format!("The audit log is unavailable: {message}").into(),
                    Some(tagged(
                        "audit-retry",
                        Button::new("audit-retry")
                            .label("Retry")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|view, _, _, cx| view.reload(cx))),
                    )),
                ),
                (Phase::Loading, _, true) => {
                    ("audit-loading", "Loading the audit log…".into(), None)
                }
                (Phase::Ready, None, true) if self.filters.is_empty() => (
                    "audit-empty",
                    "No audit records yet. Every change Oxikube makes to a cluster is listed here."
                        .into(),
                    None,
                ),
                (Phase::Ready, None, true) => (
                    "audit-no-match",
                    "No records match these filters.".into(),
                    Some(tagged(
                        "audit-clear-empty",
                        Button::new("audit-clear-empty")
                            .label("Clear filters")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|view, _, _, cx| view.clear_filters(cx))),
                    )),
                ),
                _ => return None,
            };
        Some(
            v_flex()
                .debug_selector(move || selector.to_owned())
                .size_full()
                .items_center()
                .justify_center()
                .gap(u(tokens.spacing.sm))
                .text_color(tokens.colors.text_muted)
                .child(message)
                .children(action)
                .into_any_element(),
        )
    }

    fn banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let message = self.error.clone().filter(|_| !self.model(cx).is_empty())?;
        let tokens = cx.tokens();
        Some(
            div()
                .debug_selector(|| "audit-banner".into())
                .flex_none()
                .px(u(tokens.spacing.md))
                .py(u(px(2.)))
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.error)
                .child(SharedString::from(format!(
                    "The audit log could not be read: {message}"
                )))
                .into_any_element(),
        )
    }

    fn status_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let model = self.model(cx);
        let count = model.len();
        let tail = if model.has_more() {
            "more below"
        } else {
            "end of log"
        };
        let text = format!(
            "{} record{} loaded · {tail}",
            thousands(count),
            if count == 1 { "" } else { "s" }
        );
        div()
            .debug_selector(|| "audit-status".into())
            .flex_none()
            .px(u(tokens.spacing.md))
            .py(u(px(3.)))
            .border_t_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .text_color(tokens.colors.text_muted)
            .child(SharedString::from(text))
            .into_any_element()
    }
}

impl Render for AuditView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let toolbar = self.toolbar(cx);
        let banner = self.banner(cx);
        let status = self.status_line(cx);
        let main: AnyElement = match self.state(cx) {
            Some(state) => state,
            None => div()
                .id("audit-table")
                .debug_selector(|| "audit-table".into())
                .size_full()
                .child(Table::new(&self.table).bordered(false))
                .into_any_element(),
        };
        let detail = self.detail_panel(cx);
        v_flex()
            .id("audit-view")
            .debug_selector(|| "audit-view".into())
            .key_context("AuditView")
            .track_focus(&self.focus)
            .size_full()
            .bg(tokens.colors.background)
            .text_color(tokens.colors.text)
            .child(toolbar)
            .children(banner)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(div().flex_1().min_w_0().h_full().child(main))
                    .children(detail),
            )
            .child(status)
    }
}
