//! The bodies shown instead of the list: loading, failed and empty.

use gpui::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, div,
};
use oxikube_ui::{
    ActiveTokens as _, Icon, IconName,
    button::Button,
    layout::{Disableable as _, v_flex},
    u,
};

use super::SourcesView;

/// Title of the empty state.
pub const EMPTY_TITLE: &str = "No kubeconfig sources";

/// What the empty state explains, one line each.
pub const EMPTY_STEPS: [&str; 4] = [
    "Oxikube lists the clusters of the kubeconfig files you add here.",
    "Add a kubeconfig file, or a folder of them, to read it where it is.",
    "Or paste a kubeconfig: Oxikube stores it in its own folder, readable by you only.",
    "Or read what kubectl reads: KUBECONFIG, else ~/.kube/config.",
];

/// Shown while the first read is running.
pub const LOADING_TEXT: &str = "Reading kubeconfig sources...";

fn centred(icon: IconName, title: SharedString, cx: &App) -> gpui::Div {
    let tokens = cx.tokens();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(u(tokens.spacing.md))
        .p(u(tokens.spacing.xxl))
        .text_color(tokens.colors.text_muted)
        .child(
            Icon::new(icon)
                .size(u(gpui::px(32.)))
                .color(tokens.colors.text_disabled),
        )
        .child(
            div()
                .text_size(u(tokens.font.heading))
                .text_color(tokens.colors.text)
                .child(title),
        )
}

/// The first read has not finished.
pub(super) fn loading(cx: &App) -> impl IntoElement {
    centred(IconName::FileCode, LOADING_TEXT.into(), cx).debug_selector(|| "sources-loading".into())
}

/// The sources could not be read.
pub(super) fn failed(message: &SharedString, cx: &App) -> impl IntoElement {
    centred(
        IconName::FileCode,
        "Could not read the kubeconfig sources".into(),
        cx,
    )
    .debug_selector(|| "sources-failed".into())
    .child(div().max_w(u(gpui::px(520.))).child(message.clone()))
}

/// The list is empty: explains what a source is and how to add one, with a button for the
/// default entry (the others are in the header).
pub(super) fn empty(busy: bool, cx: &mut Context<SourcesView>) -> impl IntoElement {
    let tokens = cx.tokens();
    centred(IconName::FileCode, EMPTY_TITLE.into(), cx)
        .debug_selector(|| "sources-empty".into())
        .child(
            v_flex()
                .max_w(u(gpui::px(520.)))
                .gap(u(tokens.spacing.sm))
                .children(EMPTY_STEPS.iter().enumerate().map(|(ix, step)| {
                    div()
                        .debug_selector(move || format!("sources-empty-step-{ix}"))
                        .child(*step)
                })),
        )
        .child(
            div().debug_selector(|| "sources-add-default".into()).child(
                Button::new("sources-add-default")
                    .label("Read KUBECONFIG or ~/.kube/config")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.add_default(cx))),
            ),
        )
}
