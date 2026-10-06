//! Drawing the YAML tab: the toolbar and the read-only editor.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window, div, px,
};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, editor, u};

use crate::detail::parts::skeleton;
use crate::detail::state::FullState;
use crate::detail::view::DetailView;

impl DetailView {
    /// The YAML tab: the toolbar over the editor, or a skeleton while the object is read in
    /// full, or why it could not be.
    pub(in crate::detail) fn yaml_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let body = self.yaml_content(window, cx);
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

    fn yaml_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let note = |selector: &'static str, text: String| {
            div()
                .debug_selector(move || selector.to_owned())
                .p(u(tokens.spacing.xl))
                .text_color(tokens.colors.text_muted)
                .child(text)
                .into_any_element()
        };
        // The text is made outside render (`refresh_yaml`); a missing text means the object is
        // not complete yet, or could not be read.
        let Some(text) = self.yaml.text.as_ref() else {
            if let FullState::Failed(message) = &self.full {
                return note(
                    "detail-yaml-error",
                    format!("The object could not be read in full: {message}"),
                );
            }
            return skeleton(&tokens, "detail-yaml-loading", &[320., 260., 300., 200.]);
        };
        let (key, result) = (text.key.clone(), text.result.clone());
        let text = match result {
            Ok(text) => text,
            Err(message) => return note("detail-yaml-error", message),
        };
        let state = match &self.yaml.editor {
            Some(state) => state.clone(),
            None => {
                let state = editor::read_only_state(editor::ReadOnly::YAML, window, cx);
                self.yaml.editor = Some(state.clone());
                state
            }
        };
        if self.yaml.pushed.as_ref() != Some(&key) {
            editor::set_text(&state, &text, window, cx);
            self.yaml.pushed = Some(key);
            #[cfg(test)]
            {
                self.yaml.pushes += 1;
            }
        }
        div()
            .debug_selector(|| "detail-yaml-editor".to_owned())
            .size_full()
            .overflow_hidden()
            .child(editor::code_view(&state).h_full())
            .into_any_element()
    }
}
