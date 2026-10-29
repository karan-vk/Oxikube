//! [`DryRunDiff`]: the guard's dry run of one object as the E10 diff view (E19-S07).
//!
//! The guard dry-runs every create, update, patch and apply before it asks (`oxikube_app`'s
//! `guard::preview`), and its confirmation dialog (`oxikube_workspace::ConfirmDialog`) shows each
//! object the server would write. This is the view it draws them with: the live object against
//! the server's result, normalised (server-owned fields ignored, keys sorted, Secret values
//! fingerprinted: the guard has masked them already) and diffed by [`ManifestDiff`], the code of
//! the diff vs live and the apply preview. The diff is computed on the background executor, so a
//! 1 MB object keeps the dialog inside the frame budget, and the rows are a `uniform_list`.
//! [`init`](crate::init) installs it in the dialog.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Task, UniformListScrollHandle, Window, div, px, uniform_list,
};
use oxikube_app::PreviewChange;
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, u};

use crate::diff::ManifestDiff;
use crate::view::live::{ROW_HEIGHT, inline_rows, summary};

/// Most diff rows shown before the list scrolls.
const VISIBLE_ROWS: usize = 14;

/// One previewed object's diff. See the module docs.
pub struct DryRunDiff {
    diff: Option<Arc<ManifestDiff>>,
    scroll: UniformListScrollHandle,
    /// The background diff; kept so it runs as long as the view lives, never cleared from inside.
    _diffing: Task<()>,
}

impl DryRunDiff {
    /// The view of `change`; the diff arrives from the background executor.
    pub fn new(change: &PreviewChange, cx: &mut Context<Self>) -> Self {
        let (live, result) = (change.live.clone(), change.result.clone());
        let diffing = cx.spawn(async move |this, cx| {
            let diff = cx
                .background_spawn(async move {
                    Arc::new(ManifestDiff::of_manifests(live.as_deref(), &result))
                })
                .await;
            this.update(cx, |view, cx| {
                view.diff = Some(diff);
                cx.notify();
            })
            .ok();
        });
        Self {
            diff: None,
            scroll: UniformListScrollHandle::new(),
            _diffing: diffing,
        }
    }

    /// The diff, once computed.
    pub fn diff(&self) -> Option<&ManifestDiff> {
        self.diff.as_deref()
    }
}

/// Installs [`DryRunDiff`] as the confirmation dialog's view of a previewed object.
pub(crate) fn init(cx: &mut App) {
    oxikube_workspace::set_preview_diff_builder(
        Rc::new(|change, _, cx| cx.new(|cx| DryRunDiff::new(change, cx)).into()),
        cx,
    );
}

impl Render for DryRunDiff {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let Some(diff) = self.diff.clone() else {
            return v_flex()
                .debug_selector(|| "dry-run-diff-pending".into())
                .text_color(colors.text_muted)
                .child("Comparing with the live object…");
        };
        let rows = diff.inline_rows().len();
        let height = u(px(ROW_HEIGHT * rows.clamp(1, VISIBLE_ROWS) as f32));
        let shown = diff.clone();
        v_flex()
            .debug_selector(|| "dry-run-diff".into())
            .gap(u(tokens.spacing.xs))
            .child(
                div()
                    .debug_selector(|| "dry-run-diff-summary".into())
                    .text_color(colors.text_muted)
                    .child(summary(&diff)),
            )
            .child(
                div()
                    .h(height)
                    .w_full()
                    .border_1()
                    .border_color(colors.border_variant)
                    .rounded(u(tokens.radius.sm))
                    .overflow_hidden()
                    .font_family(cx.mono_font_family())
                    .text_size(u(tokens.font.mono))
                    .child(
                        uniform_list(
                            "dry-run-diff-rows",
                            rows,
                            cx.processor(move |_, range: Range<usize>, _, cx| {
                                inline_rows(&shown, range, cx)
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .size_full(),
                    ),
            )
    }
}
