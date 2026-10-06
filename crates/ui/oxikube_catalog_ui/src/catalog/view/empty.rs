//! The bodies shown instead of the list: loading, failed, empty and nothing-matches.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, div,
};
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::v_flex, u};

/// Title of the empty state.
pub const EMPTY_TITLE: &str = "No clusters yet";

/// Why the catalog is empty and how to fill it, one line each. Shown under [`EMPTY_TITLE`].
pub const EMPTY_STEPS: [&str; 4] = [
    "Oxikube lists every context of your kubeconfig files.",
    "By default it reads ~/.kube/config. Add your clusters there, for example with the CLI of your cloud provider.",
    "Set the KUBECONFIG environment variable to read one or more other files.",
    "Add a kubeconfig file or folder, or paste one, from Kubeconfig sources.",
];

/// Shown while the first read is running.
pub const LOADING_TEXT: &str = "Reading kubeconfig files...";

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
    centred(IconName::Boxes, LOADING_TEXT.into(), cx).debug_selector(|| "catalog-loading".into())
}

/// The sources could not be read.
pub(super) fn failed(message: &SharedString, cx: &App) -> impl IntoElement {
    centred(
        IconName::Boxes,
        "Could not read your kubeconfig files".into(),
        cx,
    )
    .debug_selector(|| "catalog-failed".into())
    .child(div().max_w(u(gpui::px(520.))).child(message.clone()))
}

/// There is no cluster at all: explains how to add kubeconfigs.
pub(super) fn empty(cx: &App) -> impl IntoElement {
    let tokens = cx.tokens();
    centred(IconName::Boxes, EMPTY_TITLE.into(), cx)
        .debug_selector(|| "catalog-empty".into())
        .child(
            v_flex()
                .max_w(u(gpui::px(520.)))
                .gap(u(tokens.spacing.sm))
                .children(EMPTY_STEPS.iter().map(|step| div().child(*step))),
        )
}

/// A search matched nothing.
pub(super) fn no_match(query: &str, cx: &App) -> impl IntoElement {
    centred(
        IconName::Search,
        format!("No cluster matches \"{query}\"").into(),
        cx,
    )
    .debug_selector(|| "catalog-no-match".into())
}
