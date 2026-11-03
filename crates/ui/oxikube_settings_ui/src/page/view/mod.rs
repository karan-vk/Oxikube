//! [`SettingsPage`]: the GPUI entity of the settings page.
//!
//! | File | Holds |
//! |---|---|
//! | `mod.rs` | the struct, the search field's subscription, navigation and jumping to a result |
//! | `render.rs` | the frame: navigation, search field, the selected section or the results |
//! | `field.rs` | one field's row: title, layer badge, description, value |
//! | `item.rs` | the workspace `Item` |
//!
//! The per-cluster scope (E21-S06) swaps the model and index in place (`crate::page::cluster_tab`).
//!
//! Only the selected section is built into elements (or, while searching, the results), never
//! the whole schema. The model and the search index are built once when the page opens; a
//! keystroke filters the index and redraws once. A change of the settings (a hot reload, an edit)
//! redraws the page once, so the badges and values follow the file.

mod field;
mod item;
mod render;

pub use item::SETTINGS_VIEW;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle,
    Subscription, Window,
};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{SettingSource, SettingsStore};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::ItemEvent;

use super::cluster_tab::ClusterTabState;
use super::model::SettingsPageModel;
use super::search::{SearchHit, SearchIndex};
use crate::controls::Controls;

/// The settings page. See the [page module](crate::page).
pub struct SettingsPage {
    pub(crate) model: SettingsPageModel,
    pub(crate) index: SearchIndex,
    /// The section shown when there is no query.
    pub(crate) selected: usize,
    pub(crate) query: Entity<InputState>,
    /// The query the results are for (the field's text, trimmed).
    pub(crate) query_text: String,
    pub(crate) hits: Vec<SearchHit>,
    /// The field of the selected section a result jumped to, drawn highlighted.
    pub(crate) highlighted: Option<usize>,
    /// Where the badges read the layers at: `None` for the global value (the cluster tab of
    /// E21-S06 sets one).
    pub(crate) cluster: Option<ClusterId>,
    /// The per-cluster tab's state (E21-S06).
    pub(crate) tab: ClusterTabState,
    /// The controls of the fields shown (E21-S05).
    pub(crate) controls: Controls,
    focus: FocusHandle,
    pub(crate) content_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for SettingsPage {}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsPage {
    /// The page for every setting the app has registered: the model is derived from the
    /// [`SettingsStore`] global's schema (an empty page when the store is not installed).
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = cx
            .try_global::<SettingsStore>()
            .map(|store| SettingsPageModel::from_schema(&store.json_schema()))
            .unwrap_or_default();
        Self::with_model(model, window, cx)
    }

    /// The page over `model` (tests and the screenshot use a fixed one).
    pub fn with_model(
        model: SettingsPageModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let index = SearchIndex::new(&model);
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search settings (titles and descriptions)")
        });
        let typing = cx.subscribe_in(
            &query,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.set_query(&text, cx);
                }
                InputEvent::PressEnter { .. } => this.open_hit(0, window, cx),
                InputEvent::Focus | InputEvent::Blur => {}
            },
        );
        // The badges and values read the store at draw time: redraw when it changes.
        // Inputs that are not being edited follow the file.
        let settings = cx.observe_global_in::<SettingsStore>(window, |this, window, cx| {
            this.refresh_cluster_rows(cx);
            this.controls
                .sync_from_store(&this.model, this.cluster.as_ref(), window, cx);
            cx.notify();
        });
        // The page's focus goes to the search field, so typing searches at once.
        let focus = cx.focus_handle();
        let to_search = cx.on_focus(&focus, window, |this, window, cx| {
            this.query.update(cx, |input, cx| input.focus(window, cx));
        });
        Self {
            model,
            index,
            selected: 0,
            query,
            query_text: String::new(),
            hits: Vec::new(),
            highlighted: None,
            cluster: None,
            tab: ClusterTabState::default(),
            controls: Controls::default(),
            focus,
            content_scroll: ScrollHandle::new(),
            _subscriptions: vec![typing, settings, to_search],
        }
    }

    /// What the page shows.
    pub fn model(&self) -> &SettingsPageModel {
        &self.model
    }

    /// The index of the section shown when there is no query.
    pub fn selected_section(&self) -> usize {
        self.selected
    }

    /// The field a search result jumped to, in the selected section.
    pub fn highlighted_field(&self) -> Option<usize> {
        self.highlighted
    }

    /// The controls of the fields shown: their inputs, errors and unsaved values (the screenshot
    /// test drives an input through it, without the keyboard).
    pub fn controls(&self) -> &Controls {
        &self.controls
    }

    /// The query the results are for.
    pub fn query(&self) -> &str {
        &self.query_text
    }

    /// The results of the query, best first; empty without a query.
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }

    /// The search field.
    pub fn query_input(&self) -> &Entity<InputState> {
        &self.query
    }

    /// Shows the section at `ix` from its top. Ignored when there is no such section.
    pub fn select_section(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.model.sections().len() {
            return;
        }
        self.selected = ix;
        self.highlighted = None;
        self.content_scroll.scroll_to_item(0);
        cx.notify();
    }

    /// Searches for `text`. The results replace the section until the query is cleared.
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.trim();
        if text == self.query_text {
            return;
        }
        self.query_text = text.to_owned();
        self.hits = self.index.search(text);
        self.content_scroll.scroll_to_item(0);
        cx.notify();
    }

    /// Opens the result at `ix`: clears the search, shows its section and scrolls to its field,
    /// drawn highlighted. Ignored when there is no such result.
    pub fn open_hit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(ix).cloned() else {
            return;
        };
        self.query
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.query_text.clear();
        self.hits.clear();
        self.selected = hit.section;
        self.highlighted = hit.field;
        // Child 0 of the section's column is its header (after the cluster list, on the
        // per-cluster tab); field `n` is the child after it plus `n`.
        let header = self.header_offset();
        self.content_scroll
            .scroll_to_item(hit.field.map_or(0, |field| field + 1 + header));
        cx.notify();
    }

    /// The layer the value of the field at `path` comes from, at the page's location.
    pub fn source_of(&self, path: &str, cx: &App) -> SettingSource {
        cx.try_global::<SettingsStore>()
            .map_or(SettingSource::Default, |store| {
                store.field_source(path, self.cluster.as_ref())
            })
    }
}
