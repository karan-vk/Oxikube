//! The editor's toolbar: what is checked against what (the cluster, or syntax only), the problem
//! count, and the Read-only, Wrap and Diff (vs live, E10-S07) toggles. Each toggle sends its
//! `editor::*` command, like the keys and the palette.
//!
//! The bus commands carry no target: [`EditorViews`](super::EditorViews) acts on the focused
//! editor. gpui-component's `Button` keeps the focus where it was on mouse-down, so a toggle
//! first focuses its own editor's buffer: a click on one editor's toolbar never toggles another
//! editor (a split pane, or the one typed in last).

use gpui::{
    Action, AnyElement, ClickEvent, Context, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use oxikube_domain::command::Command;
use oxikube_keymap::contexts;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex};
use oxikube_ui::tooltip::tooltip_for_action;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::manifest_editor::ManifestEditor;
use super::model::Problems;
use super::{
    ConvertToJson, ConvertToYaml, ShowDiff, ToggleManagedFields, ToggleReadOnly, ToggleSecretMask,
    ToggleSoftWrap,
};
use crate::json::{Language, LanguageMode};
use crate::open::{ResourceBinding, ResourceLoad};

/// What the problem count says while `editor.validation` is off.
const VALIDATION_OFF: &str = "Validation off";

/// "2 errors · 1 warning", "No problems".
pub(crate) fn problems_text(problems: Problems) -> String {
    let plural = |n: usize, word: &str| match n {
        1 => format!("1 {word}"),
        n => format!("{n} {word}s"),
    };
    match (problems.errors, problems.warnings) {
        (0, 0) => "No problems".to_owned(),
        (e, 0) => plural(e, "error"),
        (0, w) => plural(w, "warning"),
        (e, w) => format!("{} · {}", plural(e, "error"), plural(w, "warning")),
    }
}

impl ManifestEditor {
    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let problems = self.model.problems();
        let validating = self.settings(cx).validation;
        let problems_color = if !validating {
            tokens.colors.text_muted
        } else if problems.errors > 0 {
            tokens.colors.error
        } else if problems.warnings > 0 {
            tokens.colors.warning
        } else {
            tokens.colors.text_muted
        };
        let (read_only, wrap) = {
            let editor = self.editor.read(cx);
            (editor.is_read_only(), editor.soft_wrap())
        };
        h_flex()
            .id("manifest-toolbar")
            .debug_selector(|| "manifest-toolbar".into())
            .flex_none()
            .gap(u(tokens.spacing.sm))
            .px(u(tokens.spacing.md))
            .py(u(tokens.spacing.xs))
            .items_center()
            .border_b_1()
            .border_color(tokens.colors.border_variant)
            .text_size(u(tokens.font.small))
            .child(Icon::new(IconName::FileCode).color(tokens.colors.text_muted))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(tokens.colors.text_muted)
                    .child(self.checked_against(cx)),
            )
            .children(self.resource_notice(cx))
            .child(div().flex_1())
            .child(
                div()
                    .id("manifest-problems")
                    .debug_selector(|| "manifest-problems".into())
                    .flex_none()
                    .text_color(problems_color)
                    .child(if validating {
                        problems_text(problems)
                    } else {
                        VALIDATION_OFF.to_owned()
                    }),
            )
            .children(self.resource_toggles(cx))
            .child(self.toggle(
                "manifest-read-only",
                "Read-only",
                &ToggleReadOnly,
                read_only,
                Command::EditorToggleReadOnly,
                false,
                cx,
            ))
            .child(self.toggle(
                "manifest-wrap",
                "Wrap",
                &ToggleSoftWrap,
                wrap,
                Command::EditorToggleSoftWrap,
                false,
                cx,
            ))
            .child(self.toggle(
                "manifest-diff-toggle",
                "Diff",
                &ShowDiff,
                self.diff_open(),
                Command::EditorShowDiff,
                false,
                cx,
            ))
            .child(self.language_chip(cx))
            .child(self.convert_button(cx))
    }

    /// The language the buffer is read as (`YAML` / `JSON`); a click reads it as the other one
    /// (`editor::UseJson` / `editor::UseYaml`). The tooltip says whether the content chose it.
    fn language_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        let language = self.model.language();
        let other = language.other();
        let how = match self.model.mode() {
            LanguageMode::Auto => "detected from the text",
            LanguageMode::Fixed(_) => "set by you",
        };
        let command = match other {
            Language::Json => Command::EditorUseJson,
            Language::Yaml => Command::EditorUseYaml,
        };
        let title = self.title.clone();
        div()
            .id("manifest-language")
            .flex_none()
            .debug_selector(move || format!("manifest-language-{title}"))
            .child(
                Button::new("manifest-language")
                    .label(language.label())
                    .ghost()
                    .xsmall()
                    .tooltip(format!(
                        "Reading as {language} ({how}). Click to read as {other}."
                    ))
                    .on_click(Self::focus_and_send(command, cx)),
            )
            .into_any_element()
    }

    /// "To JSON" / "To YAML": converts the buffer to the other language (`editor::ConvertToJson`
    /// / `editor::ConvertToYaml`); its tooltip names the key.
    fn convert_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let to = self.model.language().other();
        let (action, command): (&dyn Action, Command) = match to {
            Language::Json => (&ConvertToJson, Command::EditorConvertToJson),
            Language::Yaml => (&ConvertToYaml, Command::EditorConvertToYaml),
        };
        let title = self.title.clone();
        div()
            .id("manifest-convert")
            .flex_none()
            .debug_selector(move || format!("manifest-convert-{title}"))
            .tooltip(tooltip_for_action(
                format!("Convert to {to}"),
                action,
                Some(contexts::MANIFEST_EDITOR),
            ))
            .child(
                Button::new("manifest-convert")
                    .label(format!("To {to}"))
                    .ghost()
                    .xsmall()
                    .on_click(Self::focus_and_send(command, cx)),
            )
            .into_any_element()
    }

    /// The line after the subject: why the buffer is read-only, that the object is being read,
    /// what failed.
    fn resource_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let binding = self.binding.as_ref()?;
        let tokens = cx.tokens();
        let (text, color) = match (binding.load(), binding.notice()) {
            (ResourceLoad::Loading, _) => {
                ("Reading the object...".into(), tokens.colors.text_muted)
            }
            (ResourceLoad::Failed(_), Some(notice)) => (notice.clone(), tokens.colors.error),
            (_, Some(notice)) => (notice.clone(), tokens.colors.warning),
            _ => return None,
        };
        Some(
            div()
                .id("manifest-notice")
                .debug_selector(|| "manifest-notice".into())
                .min_w_0()
                .truncate()
                .text_color(color)
                .child(text)
                .into_any_element(),
        )
    }

    /// The managedFields and Secret-values toggles of an editor opened from an object. Off while
    /// the object is being read and while the buffer has edits (showing another rendering would
    /// throw them away).
    fn resource_toggles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(binding) = self.binding.as_ref() else {
            return Vec::new();
        };
        let locked = *binding.load() != ResourceLoad::Ready || self.is_dirty(cx);
        let mut toggles = Vec::new();
        if binding.has_managed_fields() {
            toggles.push(self.toggle(
                "manifest-managed-fields",
                "managedFields",
                &ToggleManagedFields,
                binding.options().managed_fields,
                Command::EditorToggleManagedFields,
                locked,
                cx,
            ));
        }
        if binding.is_secret() {
            let revealed = binding.options().reveal_secrets;
            toggles.push(self.toggle(
                "manifest-secret-values",
                if revealed {
                    "Hide values"
                } else {
                    "Reveal values"
                },
                &ToggleSecretMask,
                revealed,
                Command::EditorToggleSecretMask,
                locked,
                cx,
            ));
        }
        toggles
    }

    /// "kind-oxikube schemas", "Syntax only (no cluster)", plus a schema that could not be had.
    fn checked_against(&self, cx: &gpui::App) -> SharedString {
        let subject = self
            .binding
            .as_ref()
            .map_or_else(|| self.title.clone(), ResourceBinding::subject);
        let base = match &self.schemas {
            Some(source) => format!("{subject} · {} schemas", source.label(cx)),
            None => format!("{subject} · syntax only (no cluster)"),
        };
        match self.model.unavailable().first() {
            Some((gvk, _)) => format!("{base} · no schema for {gvk}").into(),
            None => base.into(),
        }
    }

    /// The click handler of a toolbar button: focuses this editor's buffer, then sends `command`
    /// (see the module docs).
    fn focus_and_send(
        command: Command,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |view, _, window, cx| {
            window.focus(&view.editor.focus_handle(cx), cx);
            view.send(command.clone(), cx);
        })
    }

    /// A toggle button; its tooltip names the key bound to the same action. Tagged
    /// `<id>-<title>` for `debug_bounds` ("manifest-wrap-Untitled-1"). A click acts on this editor
    /// ([`focus_and_send`](Self::focus_and_send)).
    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        action: &dyn Action,
        on: bool,
        command: Command,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self.title.clone();
        div()
            .id(id)
            .flex_none()
            .debug_selector(move || format!("{id}-{title}"))
            .tooltip(tooltip_for_action(
                label,
                action,
                Some(contexts::MANIFEST_EDITOR),
            ))
            .child(
                Button::new(id)
                    .label(label)
                    .ghost()
                    .xsmall()
                    .selected(on)
                    .disabled(disabled)
                    .on_click(Self::focus_and_send(command, cx)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_read_naturally() {
        let p = |errors, warnings| problems_text(Problems { errors, warnings });
        assert_eq!(p(0, 0), "No problems");
        assert_eq!(p(1, 0), "1 error");
        assert_eq!(p(0, 2), "2 warnings");
        assert_eq!(p(2, 1), "2 errors · 1 warning");
    }
}
