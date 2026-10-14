//! [`NotificationStatusItem`]: the bell and unread count in the status bar; a click toggles the
//! notification centre (`notifications::TogglePanel`).

use std::rc::Rc;

use gpui::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use oxikube_domain::command::Command;
use oxikube_ui::{ActiveTokens as _, Icon, IconName, layout::h_flex, tooltip::Tooltip, u};

use super::centre::NotificationCentre;
use crate::cluster_tab::CommandDispatcher;
use crate::status_bar::StatusItem;

/// The status bar's notification badge: always shown, the count only while something is unread.
/// It redraws only when the unread count changes.
pub struct NotificationStatusItem {
    unread: usize,
    dispatcher: Rc<dyn CommandDispatcher>,
    _observe: Subscription,
}

impl NotificationStatusItem {
    /// The badge of `centre`, toggling the panel through `dispatcher`.
    pub fn new(
        centre: &Entity<NotificationCentre>,
        dispatcher: Rc<dyn CommandDispatcher>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(centre, |this: &mut Self, centre, cx| {
            let unread = centre.read(cx).unread();
            if unread != this.unread {
                this.unread = unread;
                cx.notify();
            }
        });
        Self {
            unread: centre.read(cx).unread(),
            dispatcher,
            _observe: observe,
        }
    }

    /// The count shown.
    pub fn unread(&self) -> usize {
        self.unread
    }
}

impl StatusItem for NotificationStatusItem {
    fn visible(&self, _: &App) -> bool {
        true
    }
}

impl Render for NotificationStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let unread = self.unread;
        let tip = match unread {
            0 => "Notifications".to_owned(),
            1 => "Notifications: 1 unread".to_owned(),
            n => format!("Notifications: {n} unread"),
        };
        h_flex()
            .id("status-notifications")
            .debug_selector(|| "status-notifications".to_owned())
            .gap(u(tokens.spacing.xs))
            .px(u(tokens.spacing.sm))
            .h_full()
            .cursor_pointer()
            .text_color(tokens.colors.text_muted)
            .hover(|this| this.bg(tokens.colors.element_hover))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| {
                this.dispatcher
                    .dispatch(Command::NotificationsTogglePanel, cx);
            }))
            .child(
                Icon::new(IconName::Bell)
                    .size(u(px(13.)))
                    .color(if unread > 0 {
                        tokens.colors.accent
                    } else {
                        tokens.colors.text_muted
                    }),
            )
            .when(unread > 0, |this| {
                this.child(
                    div()
                        .debug_selector(|| "status-notifications-count".to_owned())
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.text)
                        .child(if unread > 99 {
                            "99+".to_owned()
                        } else {
                            unread.to_string()
                        }),
                )
            })
    }
}
