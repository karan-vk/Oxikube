//! Keeping the rows current: the manager's change stream, and a one second sample of the uptime
//! and the traffic counters.

use std::time::Duration;

use gpui::{Context, SharedString, Window};
use oxikube_runtime::notify_coalesced;
use oxikube_ui::input::InputEvent;

use super::PortForwardPanel;
use crate::model::Row;

/// How often the counters and uptimes are sampled while the panel is displayed.
pub(super) const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

impl PortForwardPanel {
    /// Wires the filter, the change stream and the sampler, and reads the forwards.
    pub(super) fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.filter.clone();
        let subscription = cx.subscribe_in(
            &filter,
            window,
            |panel, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    panel.set_text(&text, cx);
                }
                InputEvent::PressEnter { .. } => panel.focus_table(window, cx),
                InputEvent::Focus => panel.set_editing(true, cx),
                InputEvent::Blur => panel.set_editing(false, cx),
            },
        );
        self._subscriptions.push(subscription);

        // Clicking a header or a row focuses the library's table; the keys live on the panel.
        let refocus = cx.on_focus(&self.table.focus_handle(cx), window, |panel, window, cx| {
            window.focus(&panel.focus, cx);
        });
        self._subscriptions.push(refocus);
        let weak = cx.entity().downgrade();
        let events = self.table.on_event(cx, move |event, cx| {
            weak.update(cx, |panel, cx| panel.on_table_event(event, cx))
                .ok();
        });
        self._subscriptions.push(events);

        let mut changes = self.deps.source.subscribe();
        let follow = cx.spawn(async move |this, cx| {
            while changes.changed().await {
                if this.update(cx, |panel, cx| panel.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        let sample = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SAMPLE_INTERVAL).await;
                let alive = this.update(cx, |panel, cx| {
                    if panel.active {
                        panel.refresh(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        self.tasks.push(follow);
        self.tasks.push(sample);
        self.refresh(cx);
    }

    /// Reads the manager's forwards into the model; redraws (at frame cadence) only when a cell
    /// would show something different.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let now = self.deps.clock.now();
        let rows: Vec<Row> = self
            .deps
            .source
            .list()
            .iter()
            .map(|info| Row::new(info, self.cluster_label(&info.spec.target.cluster), now))
            .collect();
        if self.table.update_quiet(cx, |d| d.model.set_rows(rows)) {
            notify_coalesced(cx);
        }
    }

    /// The cluster as the user knows it: its context's name.
    fn cluster_label(&self, cluster: &oxikube_domain::ids::ClusterId) -> SharedString {
        match self.deps.sessions.get(cluster) {
            Some(session) => session.context().to_string().into(),
            None => cluster.to_string().into(),
        }
    }

    fn set_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
        if self.editing != editing {
            self.editing = editing;
            cx.notify();
        }
    }

    /// The text filter's new text (typed, or put there by the panel).
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.table.update_quiet(cx, |d| d.model.set_text(text)) {
            cx.notify();
        }
    }

    /// Shows the next cluster's forwards, then all of them again.
    pub fn cycle_cluster(&mut self, cx: &mut Context<Self>) {
        self.table.update_quiet(cx, |d| d.model.cycle_cluster());
        cx.notify();
    }
}
