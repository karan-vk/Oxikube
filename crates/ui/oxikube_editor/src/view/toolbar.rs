//! The editor's toolbar: what is checked against what (the cluster, or syntax only), the problem
//! count, and the Read-only and Wrap toggles. Each toggle sends its `editor::*` command, like the
//! keys and the palette.
//!
//! The bus commands carry no target: [`EditorViews`](super::EditorViews) acts on the focused
//! editor. gpui-component's `Button` keeps the focus where it was on mouse-down, so a toggle
//! first focuses its own editor's buffer: a click on one editor's toolbar never toggles another
//! editor (a split pane, or the one typed in last).

use gpui::{
    Action, AnyElement, Context, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
};
use oxikube_domain::command::Command;
use oxikube_keymap::contexts;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Selectable as _, h_flex};
use oxikube_ui::tooltip::tooltip_for_action;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::manifest_editor::ManifestEditor;
use super::model::Problems;
use super::{ToggleReadOnly, ToggleSoftWrap};

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
        let problems_color = if problems.errors > 0 {
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
            .child(div().flex_1())
            .child(
                div()
                    .id("manifest-problems")
                    .debug_selector(|| "manifest-problems".into())
                    .flex_none()
                    .text_color(problems_color)
                    .child(problems_text(problems)),
            )
            .child(self.toggle(
                "manifest-read-only",
                "Read-only",
                &ToggleReadOnly,
                read_only,
                Command::EditorToggleReadOnly,
                cx,
            ))
            .child(self.toggle(
                "manifest-wrap",
                "Wrap",
                &ToggleSoftWrap,
                wrap,
                Command::EditorToggleSoftWrap,
                cx,
            ))
    }

    /// "kind-oxikube schemas", "Syntax only (no cluster)", plus a schema that could not be had.
    fn checked_against(&self, cx: &gpui::App) -> SharedString {
        let base = match &self.schemas {
            Some(source) => format!("{} · {} schemas", self.title, source.label(cx)),
            None => format!("{} · syntax only (no cluster)", self.title),
        };
        match self.model.unavailable().first() {
            Some((gvk, _)) => format!("{base} · no schema for {gvk}").into(),
            None => base.into(),
        }
    }

    /// A toggle button; its tooltip names the key bound to the same action. Tagged
    /// `<id>-<title>` for `debug_bounds` ("manifest-wrap-Untitled-1"). A click focuses this
    /// editor's buffer before sending `command`, so the toggle acts on this editor (see the
    /// module docs).
    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        action: &dyn Action,
        on: bool,
        command: Command,
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
                    .on_click(cx.listener(move |view, _, window, cx| {
                        window.focus(&view.editor.focus_handle(cx), cx);
                        view.send(command.clone(), cx);
                    })),
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
