//! [`KeymapEditorView`]: the entity of the keymap editor page and its workspace item.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, WeakEntity, Window,
};
use oxikube_domain::command::Command;
use oxikube_keymap::{KeymapPlatform, KeymapRow};
use oxikube_ui::IconName;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_ui::table::{RowClick, TableEvent, TableHandle, TableOptions};
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent, Workspace};

use super::delegate::KeymapDelegate;
use super::model::{ContextFilter, KeymapListModel};
use super::recorder::KeybindRecorder;

/// The deduplication key of the tab: opening the editor twice shows the open one.
pub const ITEM_KEY: &str = "keymap-editor";

/// What the page needs from outside.
#[derive(Clone)]
pub struct KeymapEditorDeps {
    /// Where the page's commands go (the bus): "Open keymap.json", and the recorder's and the
    /// Remove button's edits.
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The workspace the page is in, whose modal layer shows the key recorder (E21-S08).
    pub workspace: Option<WeakEntity<Workspace>>,
}

/// The keymap editor page. See the [module docs](super).
pub struct KeymapEditorView {
    pub(super) deps: KeymapEditorDeps,
    pub(super) table: TableHandle<KeymapDelegate>,
    pub(super) search: Entity<InputState>,
    pub(super) focus: FocusHandle,
    /// The key recorder last opened from the page, while it is open.
    pub(super) recorder: Option<WeakEntity<KeybindRecorder>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for KeymapEditorView {}

impl Focusable for KeymapEditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for KeymapEditorView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Keymap").icon(IconName::Keyboard)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}

impl KeymapEditorView {
    /// The page over the installed keymap, following its changes (hot reload of `keymap.json`,
    /// the vim layer). The rows are read here, on the UI thread: building them is a few hundred
    /// small allocations, well under a frame.
    pub fn new(deps: KeymapEditorDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let file_name = oxikube_keymap::user_keymap_file(cx)
            .and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "keymap.json".to_owned());
        let delegate = KeymapDelegate::new(
            KeymapListModel::new(oxikube_keymap::keymap_rows(cx)),
            KeymapPlatform::current(),
            file_name,
        );
        let options = TableOptions {
            sortable: false,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: false,
        };
        let table = TableHandle::with_options(delegate, options, window, cx);
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search action, context or keystroke")
        });
        let weak = cx.entity().downgrade();
        let mut subscriptions = vec![cx.subscribe_in(&search, window, Self::on_search)];
        subscriptions.push(table.on_event(cx, {
            let weak = weak.clone();
            move |event, cx| {
                weak.update(cx, |this, cx| this.on_table_event(event, cx))
                    .ok();
            }
        }));
        subscriptions.push(oxikube_keymap::subscribe_changes(cx, move |cx| {
            weak.update(cx, |this, cx| this.reload(cx)).ok();
        }));
        Self {
            deps,
            table,
            search,
            focus: cx.focus_handle(),
            recorder: None,
            _subscriptions: subscriptions,
        }
    }

    /// What the table shows and filters.
    pub fn table(&self) -> &TableHandle<KeymapDelegate> {
        &self.table
    }

    /// Reads the installed keymap again. The search, the filters, the selection (by identity)
    /// and the scroll position stay.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<KeymapRow> = oxikube_keymap::keymap_rows(cx);
        self.table
            .update(cx, |delegate| delegate.model_mut().set_rows(rows));
        cx.notify();
    }

    /// Search for `text`, as if typed.
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.table
            .update(cx, |delegate| delegate.model_mut().set_query(text));
        cx.notify();
    }

    /// Put `text` in the search input and search for it, as if the user typed it.
    pub fn type_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.set_query(text, cx);
    }

    /// Limit the list to `context`.
    pub fn set_context(&mut self, context: ContextFilter, cx: &mut Context<Self>) {
        self.table
            .update(cx, |delegate| delegate.model_mut().set_context(context));
        cx.notify();
    }

    /// Show or hide the commands nothing is bound to.
    pub fn set_show_unbound(&mut self, show: bool, cx: &mut Context<Self>) {
        self.table
            .update(cx, |delegate| delegate.model_mut().set_show_unbound(show));
        cx.notify();
    }

    /// Select the `index`th row showing and scroll it into view.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.table.update(cx, |delegate| delegate.select(index)) {
            self.table.reveal_row(index, cx);
            cx.notify();
        }
    }

    /// Move the selection by `delta` rows (the first row when nothing is selected).
    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (current, len) = self.table.read(cx, |delegate| {
            (delegate.selected_position(), delegate.model().len())
        });
        if len == 0 {
            return;
        }
        let next = match current {
            Some(at) => at.saturating_add_signed(delta).min(len - 1),
            None => 0,
        };
        self.select(next, cx);
    }

    /// Opens the user's `keymap.json` (`keymap::OpenUser`). The manifest editor has no position
    /// argument, so it opens at the top whatever row is selected.
    pub fn open_json(&mut self, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(Command::KeymapOpenUser, cx);
    }

    fn on_search(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let text = input.read(cx).value().to_string();
            self.set_query(&text, cx);
        }
    }

    fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(RowClick { row, count, .. }) = event {
            self.select(*row, cx);
            if *count >= 2 {
                self.open_json(cx);
            }
        }
    }
}
