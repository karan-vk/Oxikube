//! The transfers panel: this pod's running and recent transfers with progress, speed, cancel and
//! the error text.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div, px,
};
use oxikube_app::files::{Direction, TransferInfo, TransferState};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, Sizable as _, u};

use super::FilesView;
use super::progress::progress_text;
use crate::files::format::size_text;

/// How many transfers the panel lists (the newest).
const SHOWN: usize = 6;

impl FilesView {
    /// This pod's transfers, newest last.
    pub(crate) fn my_transfers(&self) -> Vec<TransferInfo> {
        self.deps
            .runner
            .service()
            .board()
            .snapshot()
            .into_iter()
            .filter(|info| self.is_mine(info))
            .collect()
    }

    /// The panel, or `None` when this pod has had no transfer.
    pub(crate) fn transfers_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let transfers = self.my_transfers();
        if transfers.is_empty() {
            return None;
        }
        let tokens = cx.tokens();
        let skipped = transfers.len().saturating_sub(SHOWN);
        let rows: Vec<AnyElement> = transfers
            .iter()
            .skip(skipped)
            .rev()
            .map(|info| self.transfer_row(info, cx))
            .collect();
        Some(
            v_flex()
                .id("files-transfers")
                .debug_selector(|| "files-transfers".into())
                .flex_none()
                .border_t_1()
                .border_color(tokens.colors.border_variant)
                .bg(tokens.colors.surface)
                .child(
                    h_flex()
                        .px(u(tokens.spacing.md))
                        .py(u(px(3.)))
                        .gap(u(tokens.spacing.sm))
                        .items_center()
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.text_muted)
                        .child(div().flex_1().child("Transfers"))
                        .child(
                            Button::new("files-clear-transfers")
                                .label("Clear finished")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(|view, _, _, cx| view.clear_transfers(cx))),
                        ),
                )
                .children(rows)
                .into_any_element(),
        )
    }

    fn transfer_row(&self, info: &TransferInfo, cx: &mut Context<Self>) -> AnyElement {
        let tokens = cx.tokens();
        let id = info.id;
        let (icon, tone) = match &info.state {
            TransferState::Running => (
                match info.direction {
                    Direction::Download => IconName::Download,
                    Direction::Upload => IconName::Upload,
                },
                tokens.colors.accent,
            ),
            TransferState::Done => (IconName::CircleCheck, tokens.colors.success),
            TransferState::Cancelled => (IconName::CircleX, tokens.colors.text_muted),
            TransferState::Failed(_) => (IconName::CircleAlert, tokens.colors.error),
        };
        let text: SharedString = match &info.state {
            TransferState::Running => progress_text(info).into(),
            TransferState::Done => {
                let verb = match info.direction {
                    Direction::Download => "Downloaded",
                    Direction::Upload => "Uploaded",
                };
                let note = info
                    .note
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default();
                format!("{verb} {} ({}){note}", info.local, size_text(info.bytes)).into()
            }
            TransferState::Cancelled => format!("Cancelled {}", info.local).into(),
            TransferState::Failed(message) => format!("{}: {message}", info.local).into(),
        };
        let bar = (info.state == TransferState::Running)
            .then(|| info.fraction())
            .flatten()
            .map(|fraction| {
                div().h(u(px(3.))).w_full().bg(tokens.colors.element).child(
                    div()
                        .h_full()
                        .w(gpui::relative(fraction as f32))
                        .bg(tokens.colors.accent),
                )
            });
        v_flex()
            .id(SharedString::from(format!(
                "files-transfer-{}",
                id.number()
            )))
            .debug_selector(move || format!("files-transfer-{}", id.number()))
            .px(u(tokens.spacing.md))
            .py(u(px(2.)))
            .gap(u(px(2.)))
            .child(
                h_flex()
                    .gap(u(tokens.spacing.sm))
                    .items_center()
                    .text_size(u(tokens.font.small))
                    .child(Icon::new(icon).size(u(px(13.))).color(tone))
                    .child(div().flex_1().min_w_0().child(text))
                    .children((info.state == TransferState::Running).then(|| {
                        div()
                            .debug_selector(move || format!("files-cancel-{}", id.number()))
                            .child(
                                Button::new(SharedString::from(format!(
                                    "files-cancel-{}",
                                    id.number()
                                )))
                                .label("Cancel")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.cancel_transfer(id, cx);
                                    },
                                )),
                            )
                    })),
            )
            .children(bar)
            .into_any_element()
    }
}
