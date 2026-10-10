//! The toolbar (E08-U556): one row that fits at 1024 px and at UI scale 1.5. Left to right: the
//! breadcrumb (it truncates first), the container picker (`main (1/2)` with a caret when the pod
//! has several containers), the range dropdown (`tail 1000`, `since 5m`, ...), then Search,
//! Previous, Wrap, Autoscroll and the "..." menu that holds the rest (Timestamps, JSON, Mark,
//! Copy, Send to agent, Save, Clear, Tail in terminal, Fullscreen). Every control sends its
//! `logs::*` command, like the keys.
//!
//! | File | Holds |
//! |---|---|
//! | `picker` | the container picker (or the multi-pod Sources button) and the range dropdown |
//! | `overflow` | the "..." menu: its entries as data, and the dropdown |
//! | `hint` | the strip that says "Previous" holds the last crash of a crash-looping container |
//! | `levels` | the level chips bar (JSON mode) |

mod hint;
mod levels;
mod overflow;
mod picker;

use gpui::{
    Action, AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_keymap::contexts;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex};
use oxikube_ui::tooltip::tooltip_for_action;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::LogView;
use super::actions::{Find, ToggleAutoscroll, TogglePrevious, ToggleWrap};

impl LogView {
    pub(crate) fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        h_flex()
            .id("log-toolbar")
            .debug_selector(|| "log-toolbar".into())
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                // The only part that gives way: the breadcrumb is cut with an ellipsis.
                div()
                    .debug_selector(|| "log-title".into())
                    .min_w_0()
                    .truncate()
                    .text_color(tokens.colors.text_muted)
                    .text_size(u(tokens.font.small))
                    .child(self.title()),
            )
            .child(self.divider(cx))
            .child(if self.aggregate.is_some() {
                self.sources_menu(cx)
            } else {
                self.container_selector(cx)
            })
            .child(self.range_menu(cx))
            .child(div().flex_1())
            .child(self.toggle(
                Toggle::new("log-find", "Search", &Find),
                self.search.state.is_open(),
                cx,
                |view, cx| view.request_find(cx),
            ))
            .child(self.toggle(
                Toggle::new("log-previous", "Previous", &TogglePrevious),
                self.options.previous,
                cx,
                |view, cx| view.request_previous(cx),
            ))
            .child(self.toggle(
                Toggle::new("log-wrap", "Wrap", &ToggleWrap),
                self.options.wrap,
                cx,
                |view, cx| view.request_wrap(cx),
            ))
            .child(self.toggle(
                Toggle::new("log-autoscroll", "Autoscroll", &ToggleAutoscroll),
                self.follow.is_on(),
                cx,
                |view, cx| view.request_autoscroll(cx),
            ))
            .child(self.overflow_menu(cx))
    }

    /// A hairline between the breadcrumb and the controls.
    fn divider(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        div()
            .flex_none()
            .w(px(1.))
            .h(u(px(14.)))
            .bg(tokens.colors.border_variant)
            .into_any_element()
    }

    /// `namespace/pod`, or `namespace/deployment/web` for a multi-pod view.
    fn title(&self) -> String {
        let name = self.subject();
        match &self.target.namespace {
            Some(namespace) => format!("{namespace}/{name}"),
            None => name,
        }
    }

    /// A toggle button of the primary row. Its tooltip names the key that does the same
    /// (E11-S10): the binding is read from the keymap when the tooltip opens, so a rebind shows.
    fn toggle(
        &self,
        toggle: Toggle,
        on: bool,
        cx: &mut Context<Self>,
        request: fn(&mut LogView, &mut Context<LogView>),
    ) -> AnyElement {
        let Toggle { id, label, action } = toggle;
        div()
            .id(id)
            .flex_none()
            .debug_selector(move || id.to_owned())
            .tooltip(tooltip_for_action(
                label,
                action.as_ref(),
                Some(contexts::LOGS),
            ))
            .child(
                Button::new(id)
                    .label(label)
                    .ghost()
                    .xsmall()
                    .selected(on)
                    .on_click(cx.listener(move |view, _, _, cx| request(view, cx))),
            )
            .into_any_element()
    }
}

/// What a toggle of the primary row is: its id, its label and the action its key runs.
struct Toggle {
    id: &'static str,
    label: &'static str,
    action: Box<dyn Action>,
}

impl Toggle {
    fn new(id: &'static str, label: &'static str, action: &dyn Action) -> Self {
        Self {
            id,
            label,
            action: action.boxed_clone(),
        }
    }
}
