//! The "changed on server" banner over the buffer (Merge, Reload, Keep mine). Every button sends
//! its `editor::*` command, like the keys and the palette.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, prelude::FluentBuilder as _,
};
use oxikube_domain::command::Command;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{Disableable as _, h_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use crate::diff::{LiveObject, SyncState};
use crate::view::ManifestEditor;

impl ManifestEditor {
    /// The banner, while the server's change waits for a decision.
    pub(in crate::view) fn banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.live.sync.state();
        if !matches!(state, SyncState::Banner | SyncState::Merging) {
            return None;
        }
        let tokens = cx.tokens();
        let gone = matches!(self.live.sync.live(), Some(LiveObject::Gone));
        let what = self
            .live
            .target_label()
            .unwrap_or_else(|| "The object".to_owned());
        let text: SharedString = match (state, gone) {
            (_, true) => format!("{what} was deleted on the server.").into(),
            (SyncState::Merging, false) => {
                let left = self.live.sync.pending_merge().map_or(0, |m| m.unresolved());
                match left {
                    0 => "Every conflict has a side: accept the merge.".into(),
                    1 => "Pick a side for the conflict below, then accept the merge.".into(),
                    n => format!("Pick a side for the {n} conflicts below, then accept.").into(),
                }
            }
            _ => format!("{what} changed on the server while you were editing.").into(),
        };
        let merging = state == SyncState::Merging;
        let all_settled = self
            .live
            .sync
            .pending_merge()
            .is_some_and(|m| m.unresolved() == 0);
        Some(
            h_flex()
                .id("manifest-banner")
                .debug_selector(|| "manifest-banner".into())
                .flex_none()
                .gap(u(tokens.spacing.sm))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.xs))
                .items_center()
                .bg(tokens.colors.warning.opacity(0.14))
                .border_b_1()
                .border_color(tokens.colors.warning.opacity(0.5))
                .text_size(u(tokens.font.small))
                .child(Icon::new(IconName::TriangleAlert).color(tokens.colors.warning))
                .child(div().flex_1().min_w_0().truncate().child(text))
                .when(merging, |row| {
                    let accept = Command::EditorAcceptMerge;
                    row.child(self.banner_button(
                        "manifest-accept",
                        "Accept merge",
                        all_settled,
                        accept,
                        cx,
                    ))
                })
                .when(!merging, |row| {
                    let merge = Command::EditorMergeFromServer;
                    row.child(self.banner_button("manifest-merge", "Merge", !gone, merge, cx))
                })
                .child(self.banner_button(
                    "manifest-reload",
                    "Reload",
                    !gone,
                    Command::EditorReloadFromServer,
                    cx,
                ))
                .child(self.banner_button(
                    "manifest-keep",
                    "Keep mine",
                    true,
                    Command::EditorKeepMine,
                    cx,
                ))
                .into_any_element(),
        )
    }

    /// A banner button sending `command`; tagged `id` for `debug_bounds`.
    fn banner_button(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        command: Command,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex_none()
            .debug_selector(move || id.to_owned())
            .child(
                Button::new(id)
                    .label(label)
                    .xsmall()
                    .ghost()
                    .disabled(!enabled)
                    .on_click(self.sends(command, cx)),
            )
            .into_any_element()
    }

    /// A click handler that focuses this editor's buffer, then sends `command` (the bus
    /// commands act on the focused editor; see the toolbar's docs).
    pub(super) fn sends(
        &self,
        command: Command,
        cx: &mut Context<Self>,
    ) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
        cx.listener(move |view, _, window, cx| {
            view.focus_buffer(window, cx);
            view.send(command.clone(), cx);
        })
    }
}
