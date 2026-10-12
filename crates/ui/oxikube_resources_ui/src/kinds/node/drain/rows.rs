//! The drain dialog's rows: the plan before it starts, the per-pod progress while it runs.
//! Built off the render path (when the flags change, when a batch of steps arrives); a frame
//! only draws the visible ones (`uniform_list`).

use gpui::{IntoElement, ParentElement as _, SharedString, Styled as _, div, px};
use oxikube_domain::drain::{DrainBoard, DrainPlan, PodDrainState};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{Colors, Icon, IconName, u};

/// The height of one row at 100 % zoom: uniform, as `uniform_list` needs.
pub(super) const ROW_HEIGHT: f32 = 24.0;

/// How a row is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTone {
    /// Waiting, or nothing to say.
    Muted,
    /// Under way.
    Active,
    /// Held back: a budget, or a pod that is left alone.
    Warning,
    /// Done.
    Success,
    /// Failed, or blocks the drain.
    Error,
}

/// One line of the dialog's list: a pod, a short status, and a detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainLine {
    /// `namespace/name`.
    pub pod: SharedString,
    /// "Evict", "Skip", "Blocks", or the pod's drain state ("Blocked by PDB", "Deleted", ...).
    pub status: SharedString,
    /// Why, or what the server said.
    pub detail: SharedString,
    /// How it is drawn.
    pub tone: RowTone,
}

/// The plan as lines: the pods that block the drain first (they are why it cannot start), then
/// those it evicts, then those it leaves alone.
pub fn plan_lines(plan: &DrainPlan) -> Vec<DrainLine> {
    let blocked = plan.blocked.iter().map(|blocked| DrainLine {
        pod: blocked.pod.to_string().into(),
        status: "Blocks".into(),
        detail: format!(
            "{}; {} allows it",
            blocked.reason.summary(),
            blocked.reason.flag()
        )
        .into(),
        tone: RowTone::Error,
    });
    let evict = plan.evict.iter().map(|pod| DrainLine {
        pod: pod.to_string().into(),
        status: "Evict".into(),
        detail: SharedString::default(),
        tone: RowTone::Muted,
    });
    let skipped = plan.skipped.iter().map(|skipped| DrainLine {
        pod: skipped.pod.to_string().into(),
        status: "Skip".into(),
        detail: skipped.reason.explain().into(),
        tone: RowTone::Warning,
    });
    blocked.chain(evict).chain(skipped).collect()
}

/// The board as lines: one per pod the drain evicts, in plan order, then the pods it skips.
pub fn progress_lines(board: &DrainBoard) -> Vec<DrainLine> {
    let rows = board.rows().iter().map(|row| {
        let (tone, detail) = match &row.state {
            PodDrainState::Pending => (RowTone::Muted, String::new()),
            PodDrainState::Evicting { attempt } if *attempt > 1 => {
                (RowTone::Active, format!("attempt {attempt}"))
            }
            PodDrainState::Evicting { .. } | PodDrainState::Terminating => {
                (RowTone::Active, String::new())
            }
            PodDrainState::BlockedByPdb {
                reason, retry_in, ..
            } => (
                RowTone::Warning,
                format!("{reason} Retrying in {}s.", retry_in.as_secs().max(1)),
            ),
            PodDrainState::Deleted => (RowTone::Success, String::new()),
            PodDrainState::Failed { error } => (RowTone::Error, error.clone()),
        };
        DrainLine {
            pod: row.pod.to_string().into(),
            status: row.state.label().into(),
            detail: detail.into(),
            tone,
        }
    });
    let skipped = board.skipped().iter().map(|skipped| DrainLine {
        pod: skipped.pod.to_string().into(),
        status: "Skipped".into(),
        detail: skipped.reason.explain().into(),
        tone: RowTone::Muted,
    });
    rows.chain(skipped).collect()
}

/// Line `ix` of `lines`, drawn.
pub(super) fn line_row(lines: &[DrainLine], ix: usize, colors: &Colors) -> impl IntoElement {
    let line = lines.get(ix);
    let tone = line.map_or(RowTone::Muted, |line| line.tone);
    let (icon, colour) = match tone {
        RowTone::Muted => (IconName::Circle, colors.text_muted),
        RowTone::Active => (IconName::LoaderCircle, colors.accent),
        RowTone::Warning => (IconName::TriangleAlert, colors.warning),
        RowTone::Success => (IconName::CircleCheck, colors.success),
        RowTone::Error => (IconName::CircleX, colors.error),
    };
    h_flex()
        .h(u(px(ROW_HEIGHT)))
        .w_full()
        .gap(u(px(8.)))
        .items_center()
        .text_size(u(px(12.)))
        .child(Icon::new(icon).size(u(px(14.))).color(colour))
        .child(
            div()
                .flex_none()
                .max_w(u(px(240.)))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(colors.text)
                .child(line.map(|line| line.pod.clone()).unwrap_or_default()),
        )
        .child(
            div()
                .flex_none()
                .text_color(colour)
                .child(line.map(|line| line.status.clone()).unwrap_or_default()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(colors.text_muted)
                .child(line.map(|line| line.detail.clone()).unwrap_or_default()),
        )
}
