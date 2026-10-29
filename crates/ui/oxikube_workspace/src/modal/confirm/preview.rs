//! The guard's dry run in the confirmation dialog (E19-S07): each object the server would write,
//! drawn by the E10 diff view, and the server's warnings.
//!
//! The diff view lives in `oxikube_editor`, which depends on this crate, so the editor installs
//! it here once at start-up ([`set_preview_diff_builder`]); the dialog builds one view per object
//! when it opens. The view computes the diff on the background executor, so a 1 MB object never
//! costs the UI thread more than the rows on screen. Without a builder (a test of this crate
//! alone) each object is listed by its label and what happens to it.

use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Context, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Window, div,
};
use oxikube_app::apply::Change;
use oxikube_app::{PreviewChange, object_label};
use oxikube_ui::layout::v_flex;
use oxikube_ui::{ActiveTokens as _, u};

use super::ConfirmDialog;

/// Builds the view that draws one previewed object: the live object against the server's dry-run
/// result. The editor installs its diff view (E10-S07).
pub type PreviewDiffBuilder = Rc<dyn Fn(&PreviewChange, &mut Window, &mut App) -> AnyView>;

/// The app's [`PreviewDiffBuilder`], once installed.
struct PreviewDiffs(PreviewDiffBuilder);

impl Global for PreviewDiffs {}

/// Makes `builder` draw the objects of every confirmation dialog opened from now on.
pub fn set_preview_diff_builder(builder: PreviewDiffBuilder, cx: &mut App) {
    cx.set_global(PreviewDiffs(builder));
}

/// One previewed object as the dialog shows it: its heading and, with a builder installed, its
/// diff view.
pub(super) struct PreviewEntry {
    heading: SharedString,
    diff: Option<AnyView>,
}

impl PreviewEntry {
    /// "Update ConfigMap shop/web".
    pub(super) fn heading(&self) -> &SharedString {
        &self.heading
    }
}

/// The entries of `changes`, their diff views built by the installed builder, if any.
pub(super) fn entries(
    changes: &[PreviewChange],
    window: &mut Window,
    cx: &mut App,
) -> Vec<PreviewEntry> {
    let builder = cx.try_global::<PreviewDiffs>().map(|diffs| diffs.0.clone());
    changes
        .iter()
        .map(|change| PreviewEntry {
            heading: heading(change).into(),
            diff: builder.as_ref().map(|build| build(change, window, cx)),
        })
        .collect()
}

/// "Update ConfigMap shop/web".
pub fn heading(change: &PreviewChange) -> String {
    let verb = match change.change {
        Change::Create => "Create",
        Change::Update => "Update",
        Change::Unchanged => "No change to",
    };
    format!("{verb} {}", object_label(&change.target))
}

impl ConfirmDialog {
    /// Whether the dialog shows diffs, and so is drawn wide.
    pub(super) fn shows_diffs(&self) -> bool {
        self.previews.iter().any(|entry| entry.diff.is_some())
    }

    /// The previewed objects, the count of those left out, and the server's warnings.
    pub(super) fn preview_section(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = &self.prompt;
        if self.previews.is_empty() && prompt.warnings.is_empty() {
            return None;
        }
        let tokens = cx.tokens();
        let colors = tokens.colors;
        let objects = self.previews.iter().enumerate().map(|(ix, entry)| {
            v_flex()
                .debug_selector(move || format!("confirm-change-{ix}"))
                .gap(u(tokens.spacing.xs))
                .child(
                    div()
                        .text_color(colors.text_muted)
                        .child(entry.heading.clone()),
                )
                .children(entry.diff.clone())
        });
        let omitted = (prompt.omitted > 0).then(|| {
            div()
                .debug_selector(|| "confirm-changes-omitted".to_owned())
                .text_color(colors.text_muted)
                .child(match prompt.omitted {
                    1 => "1 more object is not shown".to_owned(),
                    n => format!("{n} more objects are not shown"),
                })
        });
        let warnings = prompt.warnings.iter().enumerate().map(|(ix, warning)| {
            div()
                .debug_selector(move || format!("confirm-warning-{ix}"))
                .text_color(colors.warning)
                .child(format!("Warning: {warning}"))
        });
        Some(
            v_flex()
                .debug_selector(|| "confirm-changes".to_owned())
                .gap(u(tokens.spacing.sm))
                .children(objects)
                .children(omitted)
                .children(warnings)
                .into_any_element(),
        )
    }
}
