//! The quotas of the namespace as the server counts them: `used` against `hard`, per resource.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px,
};
use oxikube_domain::view::{QuotaLevel, QuotaLine};
use oxikube_ui::allocation_bar::AllocationBar;
use oxikube_ui::chart::Tone;
use oxikube_ui::layout::{StyledExt as _, h_flex, v_flex};
use oxikube_ui::{Tokens, u};

use super::sums::QuotaView;
use crate::kinds::allocation::muted;

/// The most lines a quota shows before the rest are summarised.
const MAX_LINES: usize = 12;
/// The width of the resource column (`requests.nvidia.com/gpu`), before the UI zoom.
const NAME_WIDTH: f32 = 150.;
/// The width of the `used / hard (n%)` column, before the UI zoom.
const TEXT_WIDTH: f32 = 138.;

/// The numbers of a quota line: `used / hard (37%)`, no percent when either side is missing.
pub(in crate::kinds::namespace) fn line_text(line: &QuotaLine) -> String {
    let percent = line
        .percent()
        .map(|p| format!(" ({p}%)"))
        .unwrap_or_default();
    format!("{}{percent}", line.text())
}

/// The "Quotas" block: a heading per quota and one line per resource it caps (name, a bar of
/// `used` over `hard`, the numbers).
pub(in crate::kinds::namespace) fn quotas_block(
    tokens: &Tokens,
    quotas: &[QuotaView],
) -> AnyElement {
    v_flex()
        .debug_selector(|| "namespace-quotas".to_owned())
        .gap(u(tokens.spacing.md))
        .children(quotas.iter().enumerate().map(|(ix, quota)| {
            let hidden = quota.lines.len().saturating_sub(MAX_LINES);
            v_flex()
                .debug_selector(move || format!("namespace-quota-{ix}"))
                .gap(u(px(2.)))
                .child(
                    h_flex()
                        .gap(u(tokens.spacing.md))
                        .child(div().font_semibold().child(quota.name.to_string()))
                        .children((!quota.scopes.is_empty()).then(|| {
                            muted(tokens, format!("scoped: {}", quota.scopes.join(", ")))
                        })),
                )
                .children(
                    quota
                        .lines
                        .iter()
                        .take(MAX_LINES)
                        .enumerate()
                        .map(|(line_ix, line)| {
                            let tone = match line.level() {
                                QuotaLevel::Ok => Tone::Good,
                                QuotaLevel::Warn => Tone::Warn,
                                QuotaLevel::Full => Tone::Critical,
                            };
                            let fill = line.ratio.map_or(0.0, |r| r.clamp(0.0, 1.0)) as f32;
                            let id = format!("namespace-quota-{ix}-{line_ix}");
                            h_flex()
                                .debug_selector({
                                    let id = id.clone();
                                    move || id
                                })
                                .items_center()
                                .gap(u(tokens.spacing.md))
                                .text_size(u(tokens.font.small))
                                .child(
                                    div()
                                        .w(u(px(NAME_WIDTH)))
                                        .flex_none()
                                        .truncate()
                                        .child(line.resource.to_string()),
                                )
                                .child(
                                    div().flex_1().children(
                                        line.ratio.is_some().then(|| {
                                            AllocationBar::new(fill).tone(tone).selector(id)
                                        }),
                                    ),
                                )
                                .child(
                                    div()
                                        .w(u(px(TEXT_WIDTH)))
                                        .flex_none()
                                        .truncate()
                                        .text_right()
                                        .text_color(tokens.colors.text_muted)
                                        .child(line_text(line)),
                                )
                        }),
                )
                .children((hidden > 0).then(|| muted(tokens, format!("and {hidden} more"))))
        }))
        .into_any_element()
}
