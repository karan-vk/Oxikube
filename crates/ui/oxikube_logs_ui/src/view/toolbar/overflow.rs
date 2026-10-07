//! The toolbar's "..." menu: everything that is not on the primary row. Each entry sends the same
//! `logs::*` command as its key (every one is bound in the `LogView` keymap context).

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, Styled as _, div,
};
use oxikube_domain::log::LogSaveScope;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::menu::{DropdownMenu as _, PopupMenuItem};
use oxikube_ui::{Icon, IconName, Sizable as _};

use crate::view::LogView;

/// What an overflow entry does: the view's `request_*`, which dispatches the command.
pub(crate) type Request = fn(&mut LogView, &mut Context<LogView>);

/// One row of the overflow menu.
#[derive(Clone, Copy)]
pub(crate) enum OverflowItem {
    /// A line between groups.
    Separator,
    /// An action; `checked` shows a tick for a toggle that is on.
    Entry {
        /// A stable name of the entry (the element id it had as a toolbar button).
        id: &'static str,
        /// The words of the row.
        label: &'static str,
        /// Whether the toggle is on.
        checked: bool,
        /// Sends the command.
        request: Request,
    },
}

impl OverflowItem {
    /// The id of an entry; `None` for a separator.
    #[cfg(test)]
    pub(crate) fn id(&self) -> Option<&'static str> {
        match self {
            OverflowItem::Entry { id, .. } => Some(id),
            OverflowItem::Separator => None,
        }
    }
}

/// A menu row that says `label` and is findable as `selector` in a test window.
pub(super) fn menu_row(
    selector: impl Into<SharedString>,
    label: impl Into<SharedString>,
) -> PopupMenuItem {
    let (selector, label) = (selector.into(), label.into());
    PopupMenuItem::element(move |_, _| {
        let selector = selector.clone();
        div()
            .debug_selector(move || selector.to_string())
            .child(label.clone())
    })
}

impl LogView {
    /// The rows of the overflow menu now: display toggles, the line actions, then the terminal
    /// and fullscreen. "JSON" is there only when the log has structured lines, "Tail in terminal"
    /// only when kubectl is installed (hidden, not disabled).
    pub(crate) fn overflow_items(&self, cx: &gpui::App) -> Vec<OverflowItem> {
        let entry = |id, label, checked, request| OverflowItem::Entry {
            id,
            label,
            checked,
            request,
        };
        let mut items = vec![entry(
            "log-timestamps",
            "Timestamps",
            self.options.timestamps,
            |view, cx| view.request_timestamps(cx),
        )];
        if self.shows_json_controls() {
            items.push(entry("log-json", "JSON", self.options.json, |view, cx| {
                view.request_json_mode(cx)
            }));
        }
        let marked = self.focused_seq().is_some_and(|seq| self.is_marked(seq));
        items.extend([
            OverflowItem::Separator,
            entry("log-mark", "Mark", marked, |view, cx| view.request_mark(cx)),
            entry("log-copy", "Copy", false, |view, cx| view.request_copy(cx)),
            entry("log-send-to-agent", "Send to agent", false, |view, cx| {
                view.request_send_to_agent(cx)
            }),
            entry("log-save", "Save", false, |view, cx| {
                view.request_save(LogSaveScope::All, cx)
            }),
            entry("log-clear", "Clear", false, |view, cx| {
                view.request_clear(cx)
            }),
            OverflowItem::Separator,
        ]);
        if self.can_tail_in_terminal() {
            items.push(entry(
                "log-tail-in-terminal",
                "Tail in terminal (kubectl)",
                false,
                |view, cx| view.request_tail_in_terminal(cx),
            ));
        }
        items.push(entry(
            "log-fullscreen",
            "Fullscreen",
            self.is_fullscreen(cx),
            |view, cx| view.request_fullscreen(cx),
        ));
        items
    }

    /// The "..." button and its menu.
    pub(crate) fn overflow_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let items = self.overflow_items(cx);
        let view = cx.entity().downgrade();
        let button = Button::new("log-overflow")
            .icon(Icon::new(IconName::Ellipsis))
            .ghost()
            .xsmall()
            .tooltip("More actions")
            .dropdown_menu(move |mut menu, _, _| {
                for item in &items {
                    menu = match *item {
                        OverflowItem::Separator => menu.item(PopupMenuItem::separator()),
                        OverflowItem::Entry {
                            id,
                            label,
                            checked,
                            request,
                        } => {
                            let view = view.clone();
                            menu.item(menu_row(id, label).checked(checked).on_click(
                                move |_, _, cx| {
                                    view.update(cx, |view, cx| request(view, cx)).ok();
                                },
                            ))
                        }
                    };
                }
                menu
            });
        div()
            .flex_none()
            .debug_selector(|| "log-overflow".into())
            .child(button)
            .into_any_element()
    }
}
