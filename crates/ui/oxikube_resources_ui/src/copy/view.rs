//! Drawing the copy dialog: the name, the target, the review (change, dropped fields, missing
//! references), the choice about an existing object, the confirmation.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_app::{CopyError, CopyPreview, object_label};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::checkbox::Checkbox;
use oxikube_ui::dialog::{Cancel, Confirm, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::layout::{Disableable as _, Selectable as _, StyledExt as _, h_flex, v_flex};
use oxikube_ui::spinner::Spinner;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};
use oxikube_workspace::modal::DIALOG_KEY_CONTEXT;

use super::dialog::{Conflict, CopyDialog, CopyStage};
use super::lines::{change_line, error_line, removed_heading, removed_shown};

impl CopyDialog {
    fn title(&self) -> SharedString {
        format!("Copy {} to…", self.source.name).into()
    }

    fn notice(id: &'static str, color: gpui::Hsla, text: String) -> AnyElement {
        h_flex()
            .debug_selector(move || id.into())
            .gap(u(px(8.)))
            .items_start()
            .text_size(u(px(12.)))
            .text_color(color)
            .child(Icon::new(IconName::TriangleAlert).size(u(px(14.))))
            .child(div().flex_1().min_w_0().child(text))
            .into_any_element()
    }

    /// The target line: "kind-b / prod".
    fn target_line(&self) -> String {
        match &self.namespace {
            Some(namespace) => format!("{} / {namespace}", self.cluster_name),
            None => self.cluster_name.to_string(),
        }
    }

    fn review_body(&self, preview: &CopyPreview, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let (shown, more) = removed_shown(&preview.removed);
        v_flex()
            .gap(u(px(10.)))
            .child(
                div()
                    .debug_selector(|| "copy-change".into())
                    .font_semibold()
                    .child(change_line(preview, &self.cluster_name)),
            )
            .child(
                v_flex()
                    .debug_selector(|| "copy-removed".into())
                    .gap(u(px(2.)))
                    .child(
                        div()
                            .text_color(colors.text_muted)
                            .child(removed_heading(&preview.removed)),
                    )
                    .children(shown.iter().map(|path| {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child(path.clone())
                    }))
                    .children((more > 0).then(|| {
                        div()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child(format!("and {more} more"))
                    })),
            )
            .children((!preview.missing.is_empty()).then(|| {
                v_flex()
                    .debug_selector(|| "copy-missing".into())
                    .gap(u(px(2.)))
                    .children(preview.missing.iter().map(|reference| {
                        Self::notice("copy-missing-row", colors.warning, reference.message())
                    }))
            }))
            .children(preview.exists().then(|| self.conflict_row(preview, cx)))
            .children(self.expected_name().map(|expected| {
                v_flex()
                    .gap(u(px(4.)))
                    .child(div().child(format!("Type {expected} to confirm")))
                    .child(
                        div()
                            .debug_selector(|| "copy-typed".into())
                            .child(Input::new(&self.typed)),
                    )
            }))
            .into_any_element()
    }

    /// The object exists: overwrite, skip or rename, and the extra confirmation for force.
    fn conflict_row(&self, preview: &CopyPreview, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let button = |id: &'static str, label: &'static str, conflict: Conflict| {
            div().debug_selector(move || id.into()).child(
                Button::new(id)
                    .label(label)
                    .small()
                    .selected(self.conflict == Some(conflict))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.choose(conflict, window, cx)),
                    ),
            )
        };
        let managers = oxikube_app::apply::conflict_managers(&preview.conflicts).join(", ");
        v_flex()
            .debug_selector(|| "copy-conflict".into())
            .gap(u(px(8.)))
            .child(Self::notice(
                "copy-exists",
                colors.warning,
                format!("{} exists on the target already.", preview.label()),
            ))
            .child(
                h_flex()
                    .gap(u(px(6.)))
                    .child(button("copy-overwrite", "Overwrite", Conflict::Overwrite))
                    .child(button("copy-skip", "Skip", Conflict::Skip))
                    .child(button("copy-rename", "Rename…", Conflict::Rename)),
            )
            .children(
                (self.conflict == Some(Conflict::Overwrite) && !preview.conflicts.is_empty()).then(
                    || {
                        div().debug_selector(|| "copy-force".into()).child(
                            Checkbox::new("copy-force-box")
                                .label(format!(
                                    "Take over {} field(s) owned by {} (force)",
                                    preview.conflicts.len(),
                                    if managers.is_empty() {
                                        "other managers"
                                    } else {
                                        &managers
                                    }
                                ))
                                .checked(self.force)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.set_force(*checked, cx);
                                })),
                        )
                    },
                ),
            )
            .into_any_element()
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.colors();
        let main = match (&self.preview, &self.error, self.stage) {
            (_, _, CopyStage::Reviewing) => h_flex()
                .debug_selector(|| "copy-loading".into())
                .gap(u(px(8.)))
                .items_center()
                .text_color(colors.text_muted)
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .color(colors.accent),
                )
                .child("Connecting and asking the target for a dry run…")
                .into_any_element(),
            (Some(preview), _, _) => self.review_body(preview, cx),
            (None, _, _) => div().into_any_element(),
        };
        let create_namespace = matches!(self.error, Some(CopyError::NamespaceMissing(_)));
        v_flex()
            .gap(u(px(12.)))
            .child(
                v_flex()
                    .gap(u(px(4.)))
                    .child(
                        div()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child(format!(
                                "{} to {}",
                                object_label(&self.source),
                                self.target_line()
                            )),
                    )
                    .child(
                        div()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child("Copy as"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "copy-name".into())
                            .child(Input::new(&self.name)),
                    ),
            )
            .child(main)
            .children(self.error.as_ref().map(|error| {
                div()
                    .debug_selector(|| "copy-error".into())
                    .text_color(colors.error)
                    .child(error_line(error))
            }))
            .children(create_namespace.then(|| {
                div()
                    .debug_selector(|| "copy-create-namespace".into())
                    .child(
                        Button::new("copy-create-namespace")
                            .label("Create the namespace")
                            .on_click(cx.listener(|this, _, _, cx| this.create_namespace(cx))),
                    )
            }))
            .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let copying = self.stage == CopyStage::Copying;
        let label: SharedString = match (copying, &self.preview, self.conflict) {
            (true, ..) => "Copying…".into(),
            (_, Some(preview), Some(Conflict::Skip)) if preview.exists() => "Skip".into(),
            (_, Some(preview), Some(Conflict::Overwrite)) if preview.exists() => "Overwrite".into(),
            _ => "Copy".into(),
        };
        DialogFooter::new()
            .child(
                Button::new("copy-cancel")
                    .label("Cancel")
                    .disabled(copying)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
            .child(
                div().debug_selector(|| "copy-confirm".into()).child(
                    Button::new("copy-confirm")
                        .label(label)
                        .primary()
                        .disabled(!self.can_copy(cx))
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                ),
            )
    }
}

impl Render for CopyDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        v_flex()
            .id("copy-dialog")
            .debug_selector(|| "copy-dialog".into())
            .key_context(DIALOG_KEY_CONTEXT)
            .track_focus(&self.focus)
            .tab_group()
            .w(u(px(640.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.submit(cx)))
            .child(DialogHeader::new().child(DialogTitle::new().child(self.title())))
            .child(self.body(cx))
            .child(self.footer(cx))
    }
}
