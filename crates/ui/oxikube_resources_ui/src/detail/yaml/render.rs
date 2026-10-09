//! Drawing the YAML tab: the toolbar and the read-only code view.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    div, prelude::FluentBuilder as _, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use crate::detail::parts::skeleton;
use crate::detail::state::FullState;
use crate::detail::view::DetailView;

impl DetailView {
    /// The YAML tab: the toolbar over the code view, or a skeleton while the object is read in
    /// full and its text made, or why it could not be.
    pub(in crate::detail) fn yaml_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let body = self.yaml_content(cx);
        v_flex()
            .id("detail-yaml")
            .debug_selector(|| "detail-yaml".to_owned())
            .size_full()
            .child(self.yaml_toolbar(cx))
            .child(div().flex_1().min_h_0().min_w_0().child(body))
            .text_size(u(tokens.font.small))
            .into_any_element()
    }

    fn yaml_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let ready = self.yaml().is_some();
        let has_managed = self
            .yaml
            .text
            .as_ref()
            .is_some_and(|text| text.has_managed_fields);
        let shown = self.yaml.managed_fields;
        let icon = |name| Icon::new(name).size(u(px(14.)));
        h_flex()
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.lg))
            .py(u(tokens.spacing.sm))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .child(
                div().debug_selector(|| "yaml-managed".to_owned()).child(
                    Button::new("yaml-managed")
                        .xsmall()
                        .ghost()
                        .icon(icon(if shown {
                            IconName::Eye
                        } else {
                            IconName::EyeOff
                        }))
                        .label("managedFields")
                        .toggled(shown)
                        .disabled(!has_managed && !shown)
                        .tooltip(if shown {
                            "Hide metadata.managedFields"
                        } else {
                            "Show metadata.managedFields"
                        })
                        .on_click(
                            cx.listener(|this, _, _, cx| this.request_toggle_managed_fields(cx)),
                        ),
                ),
            )
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "yaml-copy".to_owned()).child(
                    Button::new("yaml-copy")
                        .xsmall()
                        .ghost()
                        .icon(icon(IconName::Copy))
                        .disabled(!ready)
                        .tooltip("Copy YAML")
                        .on_click(cx.listener(|this, _, _, cx| this.request_copy_yaml(cx))),
                ),
            )
            .child(
                div().debug_selector(|| "yaml-save".to_owned()).child(
                    Button::new("yaml-save")
                        .xsmall()
                        .ghost()
                        .icon(icon(IconName::Download))
                        .disabled(!ready)
                        .tooltip("Save YAML as a file")
                        .on_click(cx.listener(|this, _, _, cx| this.request_save_yaml(cx))),
                ),
            )
    }

    fn yaml_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let note = |selector: &'static str, text: String| {
            div()
                .debug_selector(move || selector.to_owned())
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element()
        };
        if let Some(Err(message)) = self.yaml.text.as_ref().map(|text| &text.result) {
            return note("detail-yaml-error", message.clone());
        }
        if self.yaml.text.is_none()
            && let FullState::Failed(message) = &self.full
        {
            return note(
                "detail-yaml-error",
                format!("The object could not be read in full: {message}"),
            );
        }
        // The text is made and laid out off the UI thread (`refresh_yaml`, `CodeView`); the
        // skeleton covers the view until its first text is on screen. The view is mounted from
        // the start so it is measured (and wraps) before that text arrives.
        let view = self.yaml.view.clone();
        let ready = view.as_ref().is_some_and(|view| view.read(cx).is_ready());
        div()
            .relative()
            .size_full()
            .children(view.map(|view| {
                div()
                    .debug_selector(|| "detail-yaml-editor".to_owned())
                    .size_full()
                    .child(view)
            }))
            .when(!ready, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(tokens.colors.surface)
                        .child(skeleton(
                            &tokens,
                            "detail-yaml-loading",
                            &[320., 260., 300., 200.],
                        )),
                )
            })
            .into_any_element()
    }
}
