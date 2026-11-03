//! Drawing the key recorder: the recorded keys, the context, the conflicts with the choice of
//! what to do about them, the validator's verdict, and Cancel / Save.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, div, px,
};
use oxikube_keymap::edit::ConflictKind;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::dialog::{DialogDescription, DialogFooter, DialogHeader, DialogTitle};
use oxikube_ui::input::Input;
use oxikube_ui::kbd::keycap;
use oxikube_ui::layout::{Disableable as _, Selectable as _, h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::model::{ConflictChoice, Verdict};
use super::view::KeybindRecorder;
use crate::keymap::{context_label, source_label};

/// The key context of the recorder, for bindings that should not fire in it.
pub const RECORDER_KEY_CONTEXT: &str = "KeymapRecorder";

impl Render for KeybindRecorder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let save = Button::new("keymap-recorder-save")
            .label("Save")
            .primary()
            .disabled(!self.model.can_save())
            .on_click(cx.listener(|this, _, _, cx| this.save(cx)));
        let cancel = Button::new("keymap-recorder-cancel")
            .label("Cancel")
            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)));
        v_flex()
            .id("keymap-recorder")
            .debug_selector(|| "keymap-recorder".to_owned())
            .key_context(RECORDER_KEY_CONTEXT)
            .track_focus(&self.focus)
            .w(u(px(520.)))
            .gap(u(tokens.spacing.lg))
            .p(u(tokens.spacing.xxl))
            .bg(colors.elevated_surface)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(u(tokens.radius.lg))
            .shadow_lg()
            .child(
                DialogHeader::new()
                    .child(DialogTitle::new().child(SharedString::from(format!(
                        "Bind \u{201c}{}\u{201d}",
                        self.model.title()
                    ))))
                    .child(
                        DialogDescription::new()
                            .child(SharedString::from(self.model.action().to_owned())),
                    ),
            )
            .child(self.keys(cx))
            .child(self.context_field(cx))
            .children(self.conflict_list(cx))
            .children(self.problems(cx))
            .child(
                DialogFooter::new()
                    .child(
                        div()
                            .debug_selector(|| "keymap-recorder-cancel".to_owned())
                            .child(cancel),
                    )
                    .child(
                        div()
                            .debug_selector(|| "keymap-recorder-save".to_owned())
                            .child(save),
                    ),
            )
    }
}

impl KeybindRecorder {
    fn keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let recording = self.model.is_recording();
        let mut caps = h_flex()
            .id("keymap-recorder-keys")
            .debug_selector(|| "keymap-recorder-keys".to_owned())
            .min_h(u(px(40.)))
            .px(u(tokens.spacing.lg))
            .gap(u(px(6.)))
            .items_center()
            .border_1()
            .rounded(u(tokens.radius.md))
            .border_color(if recording {
                colors.border_focused
            } else {
                colors.border
            });
        if self.model.keystrokes().is_empty() {
            caps = caps.child(div().text_color(colors.text_muted).child(if recording {
                "Press the keys to bind\u{2026}"
            } else {
                "No keys recorded"
            }));
        }
        for stroke in self.model.keystrokes() {
            caps = match keycap(stroke) {
                Some(cap) => caps.child(cap),
                None => caps.child(SharedString::from(stroke.clone())),
            };
        }
        let hint = if recording {
            "Recording: every key counts, Escape and Enter too. It stops after a pause."
        } else {
            "Use Record to press the keys again."
        };
        v_flex()
            .gap(u(tokens.spacing.sm))
            .child(caps)
            .child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(u(px(12.)))
                            .text_color(colors.text_muted)
                            .child(hint),
                    )
                    .child(
                        div()
                            .debug_selector(|| "keymap-recorder-record".to_owned())
                            .child(
                                Button::new("keymap-recorder-record")
                                    .label(if recording { "Stop" } else { "Record" })
                                    .small()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if recording {
                                            this.stop(cx);
                                        } else {
                                            this.restart(window, cx);
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "keymap-recorder-clear".to_owned())
                            .child(
                                Button::new("keymap-recorder-clear")
                                    .label("Clear")
                                    .small()
                                    .disabled(self.model.keystrokes().is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn context_field(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        v_flex()
            .gap(u(tokens.spacing.sm))
            .child(
                div()
                    .text_size(u(px(12.)))
                    .text_color(tokens.colors.text_muted)
                    .child("Context (blank for everywhere), for example ResourceTable && !Editing"),
            )
            .child(
                div()
                    .debug_selector(|| "keymap-recorder-context".to_owned())
                    .child(Input::new(&self.context_input).small()),
            )
            .into_any_element()
    }

    fn conflict_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let conflicts = self.model.conflicts();
        if conflicts.is_empty() {
            return None;
        }
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let mut list = v_flex()
            .id("keymap-recorder-conflicts")
            .debug_selector(|| "keymap-recorder-conflicts".to_owned())
            .gap(u(tokens.spacing.sm))
            .child(div().text_color(colors.warning).child(SharedString::from(
                match conflicts.len() {
                    1 => "1 binding in force uses these keys here:".to_owned(),
                    n => format!("{n} bindings in force use these keys here:"),
                },
            )));
        for conflict in conflicts {
            let row = &conflict.row;
            let how = match conflict.kind {
                ConflictKind::SameKeys if conflict.overwritten => "overwritten",
                ConflictKind::SameKeys => "same keys",
                ConflictKind::ShadowsLonger => "your keys start it",
                ConflictKind::ShadowedByShorter => "its keys start yours",
            };
            list = list.child(
                h_flex()
                    .gap(u(tokens.spacing.md))
                    .text_size(u(px(12.)))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(SharedString::from(row.title.clone())),
                    )
                    .child(
                        div()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(context_label(row).to_owned())),
                    )
                    .child(div().child(SharedString::from(row.keystrokes_text())))
                    .child(div().text_color(colors.text_muted).child(source_label(row)))
                    .child(div().text_color(colors.text_muted).child(how)),
            );
        }
        if self.model.overwritten().next().is_some() {
            list = list.child(
                div()
                    .debug_selector(|| "keymap-recorder-overwritten".to_owned())
                    .text_size(u(px(12.)))
                    .text_color(colors.warning)
                    .child(
                        "Saving overwrites your binding of these keys in this context: \
                         keymap.json holds one action per keys in a context, so it cannot be kept.",
                    ),
            );
        }
        if !self.model.has_choice() {
            return Some(list.into_any_element());
        }
        let choice = self.model.choice();
        let option = |id: &'static str, label: &'static str, value: ConflictChoice| {
            div().debug_selector(move || id.to_owned()).child(
                Button::new(id)
                    .label(label)
                    .small()
                    .selected(choice == value)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(value, cx))),
            )
        };
        list = list.child(
            h_flex()
                .gap(u(tokens.spacing.md))
                .child(option(
                    "keymap-recorder-keep",
                    "Keep both",
                    ConflictChoice::KeepBoth,
                ))
                .child(option(
                    "keymap-recorder-replace",
                    "Replace them",
                    ConflictChoice::Replace,
                )),
        );
        Some(list.into_any_element())
    }

    fn problems(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let colors = cx.tokens().colors;
        let (lines, color) = match self.model.verdict() {
            Verdict::Pending => return None,
            Verdict::Writable(warnings) if warnings.is_empty() => return None,
            Verdict::Writable(warnings) => (warnings, colors.warning),
            Verdict::Blocked(errors) => (errors, colors.error),
        };
        Some(
            v_flex()
                .id("keymap-recorder-problems")
                .debug_selector(|| "keymap-recorder-problems".to_owned())
                .gap(u(px(4.)))
                .text_size(u(px(12.)))
                .text_color(color)
                .children(
                    lines
                        .iter()
                        .map(|line| div().child(SharedString::from(line.clone()))),
                )
                .into_any_element(),
        )
    }
}
