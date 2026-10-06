//! What a table without rows says: loading, nothing there, or why it cannot list.
//!
//! A first cut that tells the feed states apart; E07-S10 owns the full states and diagnostics.

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    px,
};
use oxikube_app::store::FeedState;
use oxikube_ui::{ActiveTokens as _, layout::v_flex, u};

/// The message for an empty table of `plural` in `state`.
pub fn empty_message(state: &FeedState, plural: &str) -> String {
    match state {
        FeedState::Warming => format!("Loading {plural}…"),
        FeedState::Ready => format!("No {plural}"),
        FeedState::Retrying { message } => format!("Reconnecting: {message}"),
        FeedState::Forbidden { message } => format!("Not allowed to list {plural}: {message}"),
        FeedState::Failed { message, .. } => format!("Cannot list {plural}: {message}"),
    }
}

/// The empty view.
pub fn empty_view(state: &FeedState, plural: &str, cx: &App) -> impl IntoElement + use<> {
    let colors = cx.colors();
    let text: SharedString = empty_message(state, plural).into();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .p(u(px(16.)))
        .child(
            div()
                .debug_selector(|| "resource-table-empty".into())
                .text_color(colors.text_muted)
                .text_size(u(px(13.)))
                .child(text),
        )
}
