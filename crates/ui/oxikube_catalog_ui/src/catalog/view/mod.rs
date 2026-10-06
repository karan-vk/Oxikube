//! [`CatalogView`]: the GPUI entity of the catalog home.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the struct, construction, loading, the session subscription |
//! | `handlers.rs` | what keys, clicks and the search field do |
//! | `render.rs` | the frame: header, column heads, list |
//! | `row.rs` | one row of the list |
//! | `empty.rs` | the loading, failed, empty and no-match bodies |
//! | `item.rs` | the workspace `Item` |

mod empty;
mod handlers;
mod item;
mod render;
mod row;

pub use empty::{EMPTY_STEPS, EMPTY_TITLE, LOADING_TEXT};
#[cfg(test)]
pub(crate) use row::last_used_text;

use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Task,
    UniformListScrollHandle, Window,
};
use oxikube_app::{
    CatalogEntry, ClusterCatalog, ClusterSessionManager, SessionChange, SessionUpdate,
};
use oxikube_keymap::{KeyContextBuilder, KeyContextual, contexts};
use oxikube_ports::ClockPort;
use oxikube_runtime::{NotifyCoalescedExt as _, spawn_kube};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::ItemEvent;

use super::dispatch::CommandDispatcher;
use super::model::CatalogModel;

/// What the catalog view needs from the outside. All of it is handed in, so a test builds the
/// view over fakes and the app builds it over its adapters.
#[derive(Clone)]
pub struct CatalogDeps {
    /// Reads the contexts and the user's marks on them.
    pub catalog: ClusterCatalog,
    /// The sessions whose states the badges show. Only read here: connecting goes through a
    /// command.
    pub sessions: ClusterSessionManager,
    /// Where connect, disconnect and favourite commands go.
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Tells the time for "last used".
    pub clock: Arc<dyn ClockPort>,
}

/// The catalog home view. See the [module docs](super).
pub struct CatalogView {
    pub(super) deps: CatalogDeps,
    pub(super) model: CatalogModel,
    pub(super) search: Entity<InputState>,
    pub(super) search_focused: bool,
    pub(super) focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
    /// The load in flight. A newer load replaces (and so cancels) it from outside; the task
    /// never clears its own slot.
    load: Option<Task<()>>,
    /// Re-reads the catalog when the sources change. Lives as long as the view.
    _watch_sources: Task<()>,
    /// Applies session updates. Lives as long as the view.
    _watch_sessions: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for CatalogView {}

impl Focusable for CatalogView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KeyContextual for CatalogView {
    const KEY_CONTEXT: &'static str = contexts::CATALOG;

    fn extend_key_context(&self, context: &mut KeyContextBuilder) {
        context.flag_if(self.search_focused, contexts::EDITING);
    }
}

impl CatalogView {
    /// Builds the view, focuses its search field and starts reading the catalog in the
    /// background. The first frame shows the loading state; entries arrive when the read ends.
    pub fn new(deps: CatalogDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search clusters")
                .clean_on_escape()
        });
        let subscriptions = vec![cx.subscribe_in(&search, window, Self::on_search_event)];

        // Subscribe before taking the snapshot, so no update falls between the two.
        let mut updates = deps.sessions.subscribe();
        let watch_sessions = cx.spawn(async move |this, cx| {
            while let Some(item) = updates.next().await {
                let alive = this.update(cx, |this, cx| match item {
                    Ok(update) => this.apply_session_update(update, cx),
                    // Missed some: re-read every session instead of replaying.
                    Err(_) => this.sync_session_states(cx),
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let mut changes = deps.catalog.changes();
        let watch_sources = cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    break;
                }
            }
        });

        let mut view = Self {
            deps,
            model: CatalogModel::new(),
            search,
            search_focused: false,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            load: None,
            _watch_sources: watch_sources,
            _watch_sessions: watch_sessions,
            _subscriptions: subscriptions,
        };
        view.sync_session_states(cx);
        view.reload(cx);
        view.focus_search(window, cx);
        view
    }

    /// The view model, for tests and for the status of the catalog.
    pub fn model(&self) -> &CatalogModel {
        &self.model
    }

    /// Reads the catalog again (the sources changed, or the caller asks). The previous read, if
    /// still running, is dropped.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let catalog = self.deps.catalog.clone();
        self.load = Some(cx.spawn(async move |this, cx| {
            let result = spawn_kube(&*cx, async move { catalog.load().await }).await;
            let result = result
                .map_err(|error| error.to_string())
                .and_then(|loaded| loaded.map_err(|error| error.message().to_owned()));
            this.update(cx, |this, cx| this.finish_load(result, cx))
                .ok();
        }));
    }

    fn finish_load(&mut self, result: Result<Vec<CatalogEntry>, String>, cx: &mut Context<Self>) {
        match result {
            Ok(entries) => self.model.set_entries(entries),
            Err(message) => {
                tracing::warn!(%message, "the cluster catalog could not be read");
                self.model.set_failed(message);
            }
        }
        cx.notify();
    }

    fn sync_session_states(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for session in self.deps.sessions.sessions() {
            changed |= self
                .model
                .set_state(session.id().clone(), session.state().clone());
        }
        if changed {
            cx.notify_coalesced();
        }
    }

    fn apply_session_update(&mut self, update: SessionUpdate, cx: &mut Context<Self>) {
        let changed = match update.change {
            SessionChange::StateChanged { state, .. } => {
                self.model.set_state(update.cluster.clone(), state)
            }
            SessionChange::Closed => self.model.clear_state(&update.cluster),
            _ => false,
        };
        // Sessions of clusters the catalog does not list (opened by something else) change
        // nothing on screen. Everything else redraws within a frame: a burst of updates (a
        // restore opening many sessions) costs one frame, not one per update.
        if changed && self.model.contains(&update.cluster) {
            cx.notify_coalesced();
        }
    }

    fn on_search_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.on_query_changed(cx),
            InputEvent::PressEnter { .. } => self.connect_selected(cx),
            InputEvent::Focus => {
                self.search_focused = true;
                cx.notify();
            }
            InputEvent::Blur => {
                self.search_focused = false;
                cx.notify();
            }
        }
    }
}
