//! [`HelmReleasesView`]: the entity of the Releases page, its state and its actions.

use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription,
    Task, Window,
};
use jiff::Timestamp;
use oxikube_app::helm_releases::ReleaseFilter;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{Release, ReleaseRef};
use oxikube_runtime::{notify_coalesced, spawn_kube};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_ui::table::{RowClick, TableEvent, TableHandle, TableOptions};
use oxikube_workspace::CommandDispatcher;

use super::backend::ReleasesBackend;
use super::delegate::ReleasesDelegate;
use super::detail::ReleaseDetail;
use super::model::{Load, Phase, Tab};
use crate::gate::HelmGateState;

/// How long the page waits after a change before listing again, so a burst of revision records
/// (an upgrade writes several) costs one list.
pub const SETTLE: Duration = Duration::from_millis(300);

/// What the page needs from outside. All of it is handed in, so a test builds the view over fakes
/// and the app builds it over its service and bus.
#[derive(Clone)]
pub struct ReleasesDeps {
    /// Where the data comes from.
    pub backend: Rc<dyn ReleasesBackend>,
    /// Where the buttons' commands go (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// What the Helm views may offer (the binary found or not).
    pub gate: Entity<HelmGateState>,
}

/// The Helm releases of a cluster. See the [module docs](super).
pub struct HelmReleasesView {
    pub(super) deps: ReleasesDeps,
    pub(crate) table: TableHandle<ReleasesDelegate>,
    pub(super) filter: Entity<InputState>,
    /// Whether the filter field has the focus (bare keys are text then).
    pub(super) editing: bool,
    /// The selected release's tabs.
    pub(crate) detail: Option<ReleaseDetail>,
    pub(super) focus: FocusHandle,
    /// A list is being read.
    refreshing: bool,
    /// A change arrived while a list was being read: list again when it lands.
    dirty: bool,
    list_task: Option<Task<()>>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<()> for HelmReleasesView {}

impl Focusable for HelmReleasesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A failure as the user should read it.
pub(super) fn describe(error: &oxikube_domain::OxiError) -> String {
    use oxikube_domain::ErrorKind;
    match error.kind() {
        ErrorKind::Forbidden => {
            "You are not allowed to read Helm's release records (Secrets or ConfigMaps labelled \
             owner=helm) here."
                .to_owned()
        }
        ErrorKind::Network | ErrorKind::Timeout => {
            format!("The cluster is not reachable: {}", error.message())
        }
        _ => error.message().to_owned(),
    }
}

impl HelmReleasesView {
    /// A page over `deps`; the releases are read at once and kept current.
    pub fn new(deps: ReleasesDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let options = TableOptions {
            sortable: true,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: false,
        };
        let table = TableHandle::with_options(ReleasesDelegate::new(), options, window, cx);
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter releases"));
        let mut this = Self {
            deps,
            table,
            filter,
            editing: false,
            detail: None,
            focus: cx.focus_handle(),
            refreshing: false,
            dirty: false,
            list_task: None,
            _tasks: Vec::new(),
            _subscriptions: Vec::new(),
        };
        this.wire(window, cx);
        this
    }

    fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.filter.clone();
        let typing = cx.subscribe_in(
            &filter,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.set_filter_text(&text, cx);
                }
                InputEvent::PressEnter { .. } => this.focus_table(window, cx),
                InputEvent::Focus => this.set_editing(true, cx),
                InputEvent::Blur => this.set_editing(false, cx),
            },
        );
        self._subscriptions.push(typing);

        // Clicking a header or a row focuses the library's table; the keys live on the page.
        let refocus = cx.on_focus(&self.table.focus_handle(cx), window, |this, window, cx| {
            window.focus(&this.focus, cx);
        });
        self._subscriptions.push(refocus);
        let weak = cx.entity().downgrade();
        let events = self.table.on_event(cx, move |event, cx| {
            weak.update(cx, |this, cx| this.on_table_event(event, cx))
                .ok();
        });
        self._subscriptions.push(events);
        let gate = cx.observe(&self.deps.gate.clone(), |_, _, cx| cx.notify());
        self._subscriptions.push(gate);

        self.follow_records(cx);
        self.follow_reveals(cx);
        self.refresh(cx);
    }

    /// Lists again when the release records change (an upgrade outside the app, a new namespace
    /// in scope). Nothing runs while nothing changes.
    fn follow_records(&mut self, cx: &mut Context<Self>) {
        let Some(mut changes) = self.deps.backend.watch() else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                // One list for the burst: wait for it to settle, then drop what piled up.
                cx.background_executor().timer(SETTLE).await;
                while futures::poll!(changes.next()).is_ready() {}
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        self._tasks.push(task);
    }

    /// Reads the Values and Manifest tabs again when the user reveals or hides.
    fn follow_reveals(&mut self, cx: &mut Context<Self>) {
        let mut reveals = self.deps.backend.reveal_changes();
        reveals.mark_unchanged();
        let task = cx.spawn(async move |this, cx| {
            while reveals.changed().await.is_ok() {
                if this
                    .update(cx, |this, cx| this.revealed_changed(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        self._tasks.push(task);
    }

    fn set_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
        if self.editing != editing {
            self.editing = editing;
            cx.notify();
        }
    }

    /// Reads the releases again (off the UI thread). A request while one is running waits for it.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            self.dirty = true;
            return;
        }
        self.refreshing = true;
        let read = self.deps.backend.list();
        self.list_task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_kube(cx, read).await;
            let listed = match result {
                Ok(Ok(releases)) => Ok(releases),
                Ok(Err(error)) => Err(describe(&error)),
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| this.listed(listed, cx)).ok();
        }));
    }

    fn listed(&mut self, result: Result<Vec<Release>, String>, cx: &mut Context<Self>) {
        self.refreshing = false;
        let selected = self.table.read(cx, |d| d.selected().cloned());
        match result {
            Ok(releases) => {
                let now = Timestamp::now();
                self.table.update_quiet(cx, |d| {
                    d.set_releases(releases, now);
                    d.set_phase(Phase::Ready);
                });
                self.follow_selected(selected, cx);
            }
            Err(why) => {
                // Keep the rows on screen when a refresh fails; only a first read has none.
                self.table
                    .update_quiet(cx, |d| d.set_phase(Phase::Failed(why)));
            }
        }
        self.table.refresh(cx);
        notify_coalesced(cx);
        if std::mem::take(&mut self.dirty) {
            self.refresh(cx);
        }
    }

    /// After a refresh: the detail follows its release to a new revision, or closes when the
    /// release is gone.
    fn follow_selected(&mut self, selected: Option<ReleaseRef>, cx: &mut Context<Self>) {
        let Some(selected) = selected else { return };
        let current = self.table.read(cx, |d| {
            d.position_of(&selected)
                .and_then(|row| d.release(row))
                .cloned()
                .or_else(|| {
                    // Filtered out is not gone: look in the whole list.
                    d.find(&selected).cloned()
                })
        });
        match current {
            None => self.close_detail(cx),
            Some(release) => {
                let moved = self.detail.as_ref().is_some_and(|d| {
                    d.release.revision != release.revision || d.release.updated != release.updated
                });
                if moved {
                    if let Some(detail) = &mut self.detail {
                        detail.release = release;
                        detail.invalidate();
                    }
                    self.load_shown(cx);
                } else if let Some(detail) = &mut self.detail {
                    detail.release.status = release.status;
                }
            }
        }
    }

    fn revealed_changed(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        detail.generation += 1;
        for tab in [Tab::Values, Tab::Manifest] {
            detail.tasks.remove(&tab);
            *detail.load_mut(tab) = Load::Idle;
        }
        self.load_shown(cx);
        cx.notify();
    }

    // --- filter, sort, selection ------------------------------------------------------------

    /// Applies the filter text (typed, or put there by the page).
    pub fn set_filter_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let filter = ReleaseFilter::parse(text);
        self.table.update_quiet(cx, |d| d.set_filter(filter));
        self.table.refresh(cx);
        cx.notify();
    }

    pub(super) fn on_table_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        match event {
            TableEvent::RowClicked(RowClick { row, .. }) => self.select_row(*row, cx),
            TableEvent::SortChanged { .. } => cx.notify(),
            _ => {}
        }
    }

    /// Selects the release on row `row` and opens its detail.
    pub fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(release) = self.table.read(cx, |d| d.release(row).cloned()) else {
            return;
        };
        self.open_detail(release, cx);
        self.table.reveal_row(row, cx);
    }

    /// Opens the detail of `release` (its latest revision): the header and the shown tab's
    /// skeleton draw at once, the reads run off the UI thread.
    pub fn open_detail(&mut self, release: Release, cx: &mut Context<Self>) {
        if self
            .detail
            .as_ref()
            .is_some_and(|d| d.release.release == release.release)
        {
            return;
        }
        self.conceal_shown();
        let key = release.release.clone();
        self.table.update_quiet(cx, |d| d.select(Some(key)));
        self.detail = Some(ReleaseDetail::new(release, cx));
        self.load_shown(cx);
        cx.notify();
    }

    /// Closes the detail and hides what the user revealed in it.
    pub fn close_detail(&mut self, cx: &mut Context<Self>) {
        if self.detail.is_none() {
            return;
        }
        self.conceal_shown();
        self.detail = None;
        self.table.update_quiet(cx, |d| d.select(None));
        cx.notify();
    }

    fn conceal_shown(&self) {
        if let Some(detail) = &self.detail {
            self.deps.backend.conceal(&detail.release.release);
        }
    }

    /// Moves the selection `delta` rows (the first row when none is selected).
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (count, current) = self.table.read(cx, |d| {
            (
                d.len(),
                d.selected().and_then(|release| d.position_of(release)),
            )
        });
        if count == 0 {
            return;
        }
        let next = match current {
            None => 0,
            Some(row) => row.saturating_add_signed(delta).min(count - 1),
        };
        self.select_row(next, cx);
    }

    /// Selects the first or the last row.
    pub fn select_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let count = self.table.read(cx, |d| d.len());
        if count > 0 {
            self.select_row(if last { count - 1 } else { 0 }, cx);
        }
    }

    pub(super) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn focus_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.set_editing(false, cx);
    }

    // --- detail actions ---------------------------------------------------------------------

    /// Shows `tab`, reading it when it has not been read.
    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        if detail.tab != tab {
            detail.tab = tab;
            if tab != Tab::Resources {
                detail.resources.tick = None;
            }
        }
        self.load_shown(cx);
        if tab == Tab::Resources {
            self.read_objects(cx);
            self.start_ticking(cx);
        }
        cx.notify();
    }

    /// The Values tab shows the user's values, or all of them.
    pub fn toggle_values_mode(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        detail.mode = detail.mode.toggled();
        detail.generation += 1;
        detail.tasks.remove(&Tab::Values);
        detail.values.load = Load::Idle;
        self.load_shown(cx);
        cx.notify();
    }

    /// Reveals the selected release's masked values, or hides them again: the command
    /// `helm::RevealValues`, the same one the palette and an agent run.
    pub fn toggle_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else { return };
        let release = &detail.release.release;
        let reveal = !self.deps.backend.is_revealed(release);
        self.deps.dispatcher.dispatch(
            Command::HelmRevealValues {
                cluster: release.cluster.clone(),
                namespace: release.namespace.clone(),
                name: release.name.clone(),
                reveal,
            },
            cx,
        );
    }

    /// The releases table.
    pub fn table(&self) -> &TableHandle<ReleasesDelegate> {
        &self.table
    }

    /// The release of the open detail.
    pub fn selected_release(&self) -> Option<&Release> {
        self.detail.as_ref().map(|d| &d.release)
    }
}
