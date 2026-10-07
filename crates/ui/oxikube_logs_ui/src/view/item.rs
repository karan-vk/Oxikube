//! The log view as a workspace tab: title, key, focus, key context, and releasing the stream
//! when the tab closes.

use gpui::{App, Context, EventEmitter, FocusHandle, Focusable, SharedString, Window};
use oxikube_domain::ids::ResourceRef;
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ui::IconName;
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::LogView;

/// The item key of the log view of `target`: one per object in a cluster's workspace, so opening
/// the logs of a pod whose logs are open shows that tab.
pub fn item_key(target: &ResourceRef) -> String {
    format!("logs:{target}")
}

impl EventEmitter<ItemEvent> for LogView {}

impl Focusable for LogView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KeyContextual for LogView {
    /// `LogView` (`"context": "LogView"` in the keymap files).
    const KEY_CONTEXT: &'static str = contexts::LOGS;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        context.flag_if(self.options.wrap, "wrap");
        context.flag_if(self.follow.is_on(), "autoscroll");
        context.flag_if(self.options.json, "json");
        context.flag_if(self.search.state.is_open(), "searching");
        context.flag_if(self.search.editing, contexts::EDITING);
    }
}

impl Item for LogView {
    /// `pod/container`, with `(previous)` while the previous instance is read.
    fn tab_content(&self, _: &App) -> TabContent {
        let mut title = self.target.name.to_string();
        if let Some(container) = &self.options.container {
            title.push('/');
            title.push_str(container);
        }
        if self.options.previous {
            title.push_str(" (previous)");
        }
        TabContent::new(title).icon(IconName::SquareTerminal)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(item_key(&self.target).into())
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        // Dropping the session aborts its read and closes the connection.
        self.pump = None;
        self.pod_task = None;
        self.search.scan = None;
        self.session = None;
    }
}
