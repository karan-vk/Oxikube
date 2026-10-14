//! The centre's toasts: a raised notification becomes a warning toast with an "Open" button
//! (the row's open commands), keyed by the row so a later occurrence replaces it in place.
//!
//! A sticky notification (an agent waiting for a permission answer) has a toast that does not time
//! out; it goes away when the row is read, whether by its "Open" button, the panel or the answer
//! ([`follow_toasts`] watches the centre for that).
//!
//! Capped on top of the service's own rate limit: at most [`MAX_TOASTS`] per [`TOAST_WINDOW`];
//! past it one summary toast ("12 unread notifications", "Show all") stands for the rest, and
//! the toast layer's auto-dismiss takes them all away. A burst never stacks toasts.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Entity, Subscription, WeakEntity};
use oxikube_app::notifications::NotificationId;
use oxikube_domain::command::Command;
use oxikube_ports::NotificationLevel;

use super::centre::{CentreEvent, NotificationCentre, NotificationRow};
use crate::cluster_tab::CommandDispatcher;
use crate::toast::{Toast, ToastAction, ToastLevel};
use crate::workspace::Workspace;

/// The most notification toasts shown per [`TOAST_WINDOW`].
pub const MAX_TOASTS: usize = 3;

/// The window [`MAX_TOASTS`] counts in.
pub const TOAST_WINDOW: Duration = Duration::from_secs(10);

/// The longest message a toast shows, in characters; the panel keeps the whole body.
const MAX_TOAST_CHARS: usize = 160;

/// The key of the summary toast.
pub const SUMMARY_KEY: &str = "notifications/summary";

/// Shows `centre`'s raised notifications as toasts in `workspace`. Keep the subscription as long
/// as the window lives.
pub fn follow_toasts(
    centre: &Entity<NotificationCentre>,
    workspace: WeakEntity<Workspace>,
    dispatcher: Rc<dyn CommandDispatcher>,
    cx: &mut App,
) -> Subscription {
    let shown = Rc::new(Cell::new(0_usize));
    let sticky = Rc::new(RefCell::new(HashSet::<NotificationId>::new()));
    let dismiss = dismiss_when_read(centre, workspace.clone(), sticky.clone(), cx);
    let raised = cx.subscribe(centre, move |centre, event: &CentreEvent, cx| {
        let CentreEvent::Raised { rows, more } = event;
        let Some(workspace) = workspace.upgrade() else {
            return;
        };
        let mut overflow = *more;
        for row in rows {
            if shown.get() >= MAX_TOASTS {
                overflow += 1;
                continue;
            }
            shown.set(shown.get() + 1);
            let budget = shown.clone();
            // Detached on purpose: it only gives the slot back; nothing stores or drops it.
            cx.spawn(async move |cx| {
                cx.background_executor().timer(TOAST_WINDOW).await;
                budget.set(budget.get().saturating_sub(1));
            })
            .detach();
            if row.sticky() {
                sticky.borrow_mut().insert(row.id);
            }
            let toast = row_toast(row, &dispatcher);
            workspace.update(cx, |ws, cx| ws.show_toast(toast, cx));
        }
        if overflow > 0 {
            let unread = centre.read(cx).unread();
            let toast = summary_toast(unread, &dispatcher);
            workspace.update(cx, |ws, cx| ws.show_toast(toast, cx));
        }
    });
    Subscription::join(raised, dismiss)
}

/// Takes the toast of a sticky row away once the row is read (or gone).
fn dismiss_when_read(
    centre: &Entity<NotificationCentre>,
    workspace: WeakEntity<Workspace>,
    sticky: Rc<RefCell<HashSet<NotificationId>>>,
    cx: &mut App,
) -> Subscription {
    cx.observe(centre, move |centre, cx| {
        if sticky.borrow().is_empty() {
            return;
        }
        let rows = centre.read(cx).rows();
        let handled: Vec<NotificationId> = sticky
            .borrow()
            .iter()
            .copied()
            .filter(|id| {
                rows.iter()
                    .find(|row| row.id == *id)
                    .is_none_or(|row| row.read)
            })
            .collect();
        for id in handled {
            sticky.borrow_mut().remove(&id);
            if let Some(workspace) = workspace.upgrade() {
                workspace.update(cx, |ws, cx| ws.dismiss_toast_key(&key_of(id), cx));
            }
        }
    })
}

/// The toast key of a row.
pub fn toast_key(row: &NotificationRow) -> String {
    key_of(row.id)
}

fn key_of(id: NotificationId) -> String {
    format!("notification/{id}")
}

fn row_toast(row: &NotificationRow, dispatcher: &Rc<dyn CommandDispatcher>) -> Toast {
    let level = match row.level {
        NotificationLevel::Info => ToastLevel::Info,
        NotificationLevel::Success => ToastLevel::Success,
        NotificationLevel::Warning => ToastLevel::Warning,
        NotificationLevel::Error => ToastLevel::Error,
    };
    let title = match row.cluster {
        Some(_) => format!("{} \u{b7} {}", row.title, row.cluster_label),
        None => row.title.to_string(),
    };
    let mut toast = Toast::new(level, shorten(&row.body))
        .title(title)
        .key(toast_key(row));
    toast = if row.sticky() {
        // Stays until the row is read; see `dismiss_when_read`.
        toast.persistent()
    } else {
        // Every other toast goes away by itself, an error too: the centre keeps the notification.
        toast.timeout(level.default_timeout().unwrap_or(Duration::from_secs(10)))
    };
    if row.is_openable() {
        let dispatcher = dispatcher.clone();
        let row = row.clone();
        toast = toast.action(ToastAction::new("Open", move |_, cx| {
            for command in row.open_commands() {
                dispatcher.dispatch(command, cx);
            }
        }));
    }
    toast
}

fn summary_toast(unread: usize, dispatcher: &Rc<dyn CommandDispatcher>) -> Toast {
    let dispatcher = dispatcher.clone();
    let message = match unread {
        0 | 1 => "More notifications in the notification centre".to_owned(),
        n => format!("{n} unread notifications in the notification centre"),
    };
    Toast::warning(message)
        .key(SUMMARY_KEY)
        .action(ToastAction::new("Show all", move |_, cx| {
            dispatcher.dispatch(Command::NotificationsTogglePanel, cx);
        }))
}

/// `text` cut to [`MAX_TOAST_CHARS`], ending in an ellipsis when cut.
fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_TOAST_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(MAX_TOAST_CHARS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}
