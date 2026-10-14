//! [`NotificationPanel`]: the notification centre as a right-dock panel of the window.
//!
//! A virtualised list of the rows (newest first) with the unread ones marked, a header with the
//! unread count, "Mark all read" and the shown cluster's mute toggle. A click on a row (or
//! `enter` on the selected one) marks it read, selects its cluster and opens its object; `r`
//! marks the selected row read, `shift-r` all of them (keymap context `NotificationPanel`).
//! Every one of these is a bus command dispatched through the window's dispatcher.

use std::rc::Rc;

use gpui::{
    Action, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, SharedString,
    Subscription, UniformListScrollHandle, Window, px,
};
use oxikube_app::notifications::NotificationId;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_ui::IconName;

use super::TogglePanel;
use super::centre::{NotificationCentre, NotificationRow};
use crate::cluster_tab::CommandDispatcher;
use crate::panel::{DockPosition, Panel, PanelEvent};

/// The panel's name in saved layouts.
pub const PANEL_NAME: &str = "NotificationPanel";

/// The notification centre panel. See the `panel` module docs.
pub struct NotificationPanel {
    pub(super) centre: Entity<NotificationCentre>,
    pub(super) dispatcher: Rc<dyn CommandDispatcher>,
    pub(super) focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
    /// The selected row's index (keyboard).
    pub(super) selected: usize,
    /// The row the user selected: the selection follows it when newer ones push it down.
    selected_id: Option<NotificationId>,
    /// The cluster of the shown cluster tab: what the header's mute toggle acts on.
    pub(super) active: Option<ClusterId>,
    _observe: Subscription,
}

impl NotificationPanel {
    /// A panel over `centre`, dispatching through `dispatcher`.
    pub fn new(
        centre: Entity<NotificationCentre>,
        dispatcher: Rc<dyn CommandDispatcher>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&centre, |this: &mut Self, centre, cx| {
            let rows = centre.read(cx).rows();
            let kept = this
                .selected_id
                .and_then(|id| rows.iter().position(|row| row.id == id));
            // Until the user picks a row the selection stays on the newest.
            this.selected = kept.unwrap_or(this.selected.min(rows.len().saturating_sub(1)));
            cx.notify();
        });
        Self {
            centre,
            dispatcher,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            selected: 0,
            selected_id: None,
            active: None,
            _observe: observe,
        }
    }

    /// Sets the cluster the header's mute toggle acts on (the shown cluster tab; `None` hides it).
    pub fn set_active_cluster(&mut self, cluster: Option<ClusterId>, cx: &mut Context<Self>) {
        if self.active != cluster {
            self.active = cluster;
            cx.notify();
        }
    }

    /// The cluster the mute toggle acts on.
    pub fn active_cluster(&self) -> Option<&ClusterId> {
        self.active.as_ref()
    }

    /// The selected row's index.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The model.
    pub fn centre(&self) -> &Entity<NotificationCentre> {
        &self.centre
    }

    pub(super) fn rows(&self, cx: &App) -> Rc<[NotificationRow]> {
        self.centre.read(cx).rows()
    }

    pub(super) fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.rows(cx).len();
        if len == 0 {
            return;
        }
        let next = self.selected.saturating_add_signed(delta).min(len - 1);
        if next != self.selected {
            self.selected = next;
            self.selected_id = self.rows(cx).get(next).map(|row| row.id);
            self.scroll
                .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    /// Opens row `ix`: marks it read, selects its cluster, opens its object.
    pub(super) fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows(cx).get(ix).cloned() else {
            return;
        };
        self.selected = ix;
        self.selected_id = Some(row.id);
        for command in row.open_commands() {
            self.dispatcher.dispatch(command, cx);
        }
    }

    pub(super) fn mark_selected_read(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.rows(cx).get(self.selected) {
            let id = row.id.0;
            self.dispatcher
                .dispatch(Command::NotificationsMarkRead { id }, cx);
        }
    }

    pub(super) fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        self.dispatcher
            .dispatch(Command::NotificationsMarkAllRead, cx);
    }

    /// Mutes the shown cluster, or unmutes it.
    pub(super) fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let Some(cluster) = self.active.clone() else {
            return;
        };
        let command = if self.centre.read(cx).is_muted(&cluster) {
            Command::NotificationsUnmuteCluster { cluster }
        } else {
            Command::NotificationsMuteCluster { cluster }
        };
        self.dispatcher.dispatch(command, cx);
    }

    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Close);
    }
}

impl EventEmitter<PanelEvent> for NotificationPanel {}

impl Focusable for NotificationPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Panel for NotificationPanel {
    fn persistent_name() -> &'static str {
        PANEL_NAME
    }

    fn panel_key() -> &'static str {
        "notification_panel"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(380.)
    }

    fn min_size(&self, _: &Window, _: &App) -> Option<Pixels> {
        Some(px(260.))
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::Bell)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<SharedString> {
        Some("Notifications".into())
    }

    fn title(&self, cx: &App) -> SharedString {
        match self.centre.read(cx).unread() {
            0 => "Notifications".into(),
            unread => format!("Notifications ({unread})").into(),
        }
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(TogglePanel)
    }

    fn activation_priority(&self) -> u32 {
        100
    }
}
