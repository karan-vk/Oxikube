//! The banner of a multi-pod view: a non-modal strip under the toolbar that says what changed in
//! the pod set ("pod web-7d9 added", "pod web-4c1 ended"), how many pods the stream cap left out,
//! and when no pod matches. The model is plain Rust; [`LogView::banner`] draws it.

use std::collections::VecDeque;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div,
};
use oxikube_app::logs::{PodChange, PodEvent};
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::h_flex;
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use crate::view::LogView;

/// Pod events the banner shows at once (the newest).
pub const BANNER_LINES: usize = 3;

/// How long the pod events stay after the last one, in seconds.
pub const BANNER_SECONDS: u64 = 8;

/// The pod events the banner shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Banner {
    events: VecDeque<PodEvent>,
}

impl Banner {
    /// Adds `event`; only the newest [`BANNER_LINES`] stay.
    pub fn push(&mut self, event: PodEvent) {
        self.events.push_back(event);
        while self.events.len() > BANNER_LINES {
            self.events.pop_front();
        }
    }

    /// Removes every event (dismissed, or expired).
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Whether there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// What each event says, oldest first: `pod web-7d9 added`, `pod web-4c1 ended`.
    pub fn lines(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|event| {
                let what = match event.change {
                    PodChange::Added => "added",
                    PodChange::Ended => "ended",
                };
                format!("pod {} {what}", event.pod)
            })
            .collect()
    }
}

impl LogView {
    /// The notices of a multi-pod view: pod events, pods left out by the cap, no pods. `None`
    /// when there is nothing to say.
    pub(crate) fn banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.aggregate.as_ref()?;
        let mut lines = state.banner.lines();
        if state.skipped_pods > 0 {
            lines.push(format!(
                "{} more pods not streamed (logs.max_streams = {})",
                state.skipped_pods,
                self.deps.service.max_streams()
            ));
        }
        lines.extend(self.no_pods_notice().or_else(|| self.waiting_pods_notice()));
        if lines.is_empty() {
            return None;
        }
        let tokens = cx.tokens();
        let dismissable = !state.banner.is_empty();
        let rows = lines.into_iter().map(|line| {
            div()
                .text_size(u(tokens.font.small))
                .text_color(tokens.colors.text)
                .child(line)
        });
        Some(
            h_flex()
                .id("log-banner")
                .debug_selector(|| "log-banner".into())
                .flex_none()
                .items_start()
                .gap(u(tokens.spacing.md))
                .px(u(tokens.spacing.md))
                .py(u(tokens.spacing.sm))
                .bg(tokens.colors.surface)
                .border_b_1()
                .border_color(tokens.colors.border_variant)
                .child(div().flex_1().min_w_0().children(rows))
                .children(dismissable.then(|| {
                    div().debug_selector(|| "log-banner-dismiss".into()).child(
                        Button::new("log-banner-dismiss")
                            .label("Dismiss")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|view, _, _, cx| view.dismiss_banner(cx))),
                    )
                }))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn event(seq: u64, pod: &str, change: PodChange) -> PodEvent {
        PodEvent {
            seq,
            pod: Arc::from(pod),
            change,
        }
    }

    #[test]
    fn events_read_as_pod_added_and_pod_ended_and_only_the_newest_stay() {
        let mut banner = Banner::default();
        assert!(banner.is_empty());
        banner.push(event(0, "web-7d9", PodChange::Added));
        banner.push(event(1, "web-4c1", PodChange::Ended));
        assert_eq!(banner.lines(), ["pod web-7d9 added", "pod web-4c1 ended"]);
        banner.push(event(2, "web-a", PodChange::Added));
        banner.push(event(3, "web-b", PodChange::Added));
        assert_eq!(banner.lines().len(), BANNER_LINES);
        assert_eq!(banner.lines()[0], "pod web-4c1 ended");
        banner.clear();
        assert!(banner.is_empty());
    }
}
