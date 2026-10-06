//! [`NamespaceSelector`]: the state and behaviour of the dropdown. Drawing is in `render.rs`.

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Task,
    UniformListScrollHandle, Window,
};
use oxikube_app::session::namespaces::{
    NamespaceCatalog, NamespacePrefs, NamespaceService, NamespaceSource, slot_selection,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};

use super::background::flatten;
use super::events::NamespaceSelectorEvent;
use super::model::{Row, build_rows, first_selectable, selection_label, step};

/// The namespace dropdown of one cluster. See the [module docs](super).
///
/// Everything it changes goes through the [`NamespaceService`]: ticking namespaces is the
/// `namespace::Select` command behind a 150 ms debounce, `0`-`9` and "All namespaces" are the
/// same command at once, the star is `namespace::ToggleFavourite`. The view updates itself
/// first (so input costs one frame) and tells the service off the UI thread. It never opens
/// feeds: it only changes the selection, and the store re-scopes.
pub struct NamespaceSelector {
    pub(super) cluster: ClusterId,
    pub(super) service: NamespaceService,
    pub(super) prefs: NamespacePrefs,
    pub(super) catalog: NamespaceCatalog,
    pub(super) open: bool,
    pub(super) query: SharedString,
    pub(super) rows: Vec<Row>,
    pub(super) highlighted: usize,
    pub(super) error: Option<SharedString>,
    pub(super) search: Entity<InputState>,
    pub(super) trigger_focus: FocusHandle,
    pub(super) list_focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
    /// The debounced commit of the last tick. Replacing it drops (aborts) the one before, so
    /// only the newest toggle reaches the service; it is never cleared from inside itself.
    pub(super) commit: Option<Task<()>>,
    pub(super) commit_generation: u64,
    /// A tick is waiting for its commit: the view's selection is ahead of the session's.
    pub(super) dirty: bool,
    /// Keeps the load alive; dropped with the view.
    pub(super) _load: Option<Task<()>>,
    /// Keeps the session subscription alive; dropped with the view.
    pub(super) _watch: Option<Task<()>>,
}

impl EventEmitter<NamespaceSelectorEvent> for NamespaceSelector {}

impl Focusable for NamespaceSelector {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.trigger_focus.clone()
    }
}

impl NamespaceSelector {
    /// A selector for `cluster`. It restores the remembered selection, lists the namespaces and
    /// follows the session's selection, all in the background.
    ///
    /// Needs the runtime bridge (`oxikube_runtime::init*`) and `oxikube_ui::init`.
    pub fn new(
        cluster: ClusterId,
        service: NamespaceService,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search namespaces"));
        cx.subscribe_in(
            &search,
            window,
            |this, input, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    let query = input.read(cx).value();
                    this.set_query(query, cx);
                }
                InputEvent::PressEnter { .. } => this.activate(this.highlighted, cx),
                _ => {}
            },
        )
        .detach();
        let selection = service
            .manager()
            .get(&cluster)
            .map(|s| s.namespace_selection().clone())
            .unwrap_or_default();
        let mut this = Self {
            cluster,
            service,
            prefs: NamespacePrefs {
                selection,
                ..NamespacePrefs::default()
            },
            catalog: NamespaceCatalog::unlisted(),
            open: false,
            query: SharedString::default(),
            rows: Vec::new(),
            highlighted: 0,
            error: None,
            search,
            trigger_focus: cx.focus_handle().tab_stop(true),
            list_focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            commit: None,
            commit_generation: 0,
            dirty: false,
            _load: None,
            _watch: None,
        };
        this.rebuild();
        this._watch = Some(this.watch_session(cx));
        this._load = Some(this.load(cx));
        this
    }

    // --- read side -------------------------------------------------------------------

    /// The cluster this selector belongs to.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// The selection as the view shows it (it can be ahead of the session's by the debounce).
    pub fn selection(&self) -> &NamespaceSelection {
        &self.prefs.selection
    }

    /// The favourites.
    pub fn favourites(&self) -> &NamespaceFavourites {
        &self.prefs.favourites
    }

    /// The namespaces on offer and where they came from.
    pub fn catalog(&self) -> &NamespaceCatalog {
        &self.catalog
    }

    /// The trigger's text (`All namespaces`, `prod`, `prod +2`).
    pub fn label(&self) -> String {
        selection_label(&self.prefs.selection)
    }

    /// Whether the dropdown is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The rows the dropdown lists for the current query.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The highlighted row.
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// The last error, shown at the bottom of the dropdown until the next change.
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    // --- opening and searching -------------------------------------------------------

    /// Opens the dropdown, focuses the search box and refreshes the namespace list.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.open = true;
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.query = SharedString::default();
        self.rebuild();
        self.highlighted = first_selectable(&self.rows, 0).unwrap_or(0);
        self.search.update(cx, |input, cx| input.focus(window, cx));
        self.refresh_catalog(cx);
        cx.notify();
    }

    /// Closes the dropdown and gives focus back to the trigger.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        window.focus(&self.trigger_focus, cx);
        cx.notify();
    }

    /// Opens or closes the dropdown.
    pub fn toggle_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    pub(super) fn set_query(&mut self, query: SharedString, cx: &mut Context<Self>) {
        if self.query == query {
            return;
        }
        self.query = query;
        self.rebuild();
        self.highlighted = first_selectable(&self.rows, 0).unwrap_or(0);
        self.scroll
            .scroll_to_item(self.highlighted, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn move_highlight(&mut self, direction: isize, cx: &mut Context<Self>) {
        self.highlighted = step(&self.rows, self.highlighted, direction);
        self.scroll
            .scroll_to_item(self.highlighted, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    // --- changing the selection ------------------------------------------------------

    /// Activates row `index`: picks All, ticks or unticks a namespace, or adds a typed one.
    pub fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.rows.get(index).cloned() {
            Some(Row::All { .. }) => self.select_slot(0, cx),
            Some(Row::Namespace(row)) => self.toggle_namespace(&row.name, cx),
            Some(Row::Add { name }) => self.add_typed(&name, cx),
            Some(Row::Header(_)) | None => {}
        }
    }

    /// Ticks `name` if it is unticked, unticks it otherwise. Unticking the last one gives All.
    /// Rapid calls are debounced into one change of the session.
    pub fn toggle_namespace(&mut self, name: &str, cx: &mut Context<Self>) {
        let mut selection = self.prefs.selection.clone();
        if !selection.remove(name) {
            selection.insert(name);
        }
        self.prefs.selection = selection.clone();
        self.rebuild();
        self.commit_debounced(selection, cx);
        cx.notify();
    }

    /// Selects slot `slot` at once: `0` is All, `1`-`9` the favourite with that digit. A digit
    /// with no favourite does nothing.
    pub fn select_slot(&mut self, slot: u8, cx: &mut Context<Self>) {
        let Some(selection) = slot_selection(&self.prefs.favourites, slot) else {
            return;
        };
        self.prefs.selection = selection.clone();
        self.rebuild();
        // The command supersedes a pending tick: drop it.
        self.commit = None;
        self.dirty = false;
        self.run(
            Command::NamespaceSelect {
                cluster: self.cluster.clone(),
                namespaces: selection.names().map(str::to_owned).collect(),
            },
            cx,
        );
        cx.notify();
    }

    /// Pins `name`, or unpins it when it is pinned.
    pub fn toggle_favourite(&mut self, name: &str, cx: &mut Context<Self>) {
        self.prefs.favourites.toggle(name);
        self.rebuild();
        self.run(
            Command::NamespaceToggleFavourite {
                cluster: self.cluster.clone(),
                namespace: name.to_owned(),
            },
            cx,
        );
        cx.notify();
    }

    /// Remembers a namespace the user typed (the cluster does not list its namespaces) and
    /// ticks it.
    pub fn add_typed(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.prefs.add_typed(name) {
            self.catalog.insert_typed(name);
        }
        let (service, cluster, name_owned) =
            (self.service.clone(), self.cluster.clone(), name.to_owned());
        cx.spawn(async move |this, cx| {
            let result = flatten(
                spawn_kube(
                    cx,
                    async move { service.add_typed(&cluster, &name_owned).await },
                )
                .await,
            );
            this.update(cx, |this, cx| this.answered(result, cx)).ok();
        })
        .detach();
        self.toggle_namespace(name, cx);
    }

    pub(super) fn rebuild(&mut self) {
        self.rows = build_rows(
            &self.query,
            &self.catalog,
            &self.prefs.selection,
            &self.prefs.favourites,
        );
        self.highlighted = self.highlighted.min(self.rows.len().saturating_sub(1));
    }

    /// Whether the cluster refused to list namespaces (the dropdown explains and offers typing).
    pub fn is_restricted(&self) -> bool {
        self.catalog.source == NamespaceSource::Forbidden
    }
}
