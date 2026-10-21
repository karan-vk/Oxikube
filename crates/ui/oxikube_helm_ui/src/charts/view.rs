//! [`HelmChartsView`]: the entity of the charts page, its state and its actions.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, UniformListScrollHandle, Window,
};
use oxikube_app::ChartsReadiness;
use oxikube_domain::command::Command;
use oxikube_domain::helm::ChartSummary;
use oxikube_runtime::spawn_kube;
use oxikube_ui::IconName;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent};

use super::backend::ChartsBackend;
use super::detail::Detail;
use super::model::{Card, Phase, Tab};

/// The deduplication key of the tab: opening the page twice shows the open one.
pub const ITEM_KEY: &str = "helm-charts";

/// How long typing pauses before the query is sent to `helm`.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// What the page needs from outside. All of it is handed in, so a test builds the view over fakes
/// and the app builds it over its service and bus.
#[derive(Clone)]
pub struct ChartsDeps {
    /// Where the data comes from.
    pub backend: Rc<dyn ChartsBackend>,
    /// Where the buttons' commands go (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// The Helm charts catalog. See the [module docs](super).
pub struct HelmChartsView {
    pub(super) deps: ChartsDeps,
    pub(super) phase: Phase,
    /// The charts of the last search, and their cards (same order).
    pub(super) charts: Arc<Vec<ChartSummary>>,
    pub(super) cards: Vec<Card>,
    /// A search is running.
    pub(super) searching: bool,
    /// The last search's failure.
    pub(super) problem: Option<String>,
    pub(super) query: Entity<InputState>,
    pub(super) list_scroll: UniformListScrollHandle,
    pub(super) selected: Option<usize>,
    pub(super) detail: Option<Detail>,
    pub(super) focus: FocusHandle,
    /// The readiness check; dropped (and so aborted) with the page or replaced by a new one.
    ready_task: Option<Task<()>>,
    /// The debounce and the search after it; typing again replaces (and so cancels) it.
    search_task: Option<Task<()>>,
    _watch: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ItemEvent> for HelmChartsView {}

impl Focusable for HelmChartsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for HelmChartsView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Helm Charts").icon(IconName::Package)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(ITEM_KEY.into())
    }
}

impl HelmChartsView {
    /// The page: it looks for `helm` and the repositories off the UI thread, then lists every
    /// chart. It follows repository changes.
    pub fn new(deps: ChartsDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search charts"));
        let subscription = cx.subscribe_in(&query, window, Self::on_query);
        let mut changes = deps.backend.changes();
        let watch = cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            deps,
            phase: Phase::Checking,
            charts: Arc::default(),
            cards: Vec::new(),
            searching: false,
            problem: None,
            query,
            list_scroll: UniformListScrollHandle::new(),
            selected: None,
            detail: None,
            focus: cx.focus_handle(),
            ready_task: None,
            search_task: None,
            _watch: watch,
            _subscriptions: vec![subscription],
        };
        view.reload(cx);
        view
    }

    /// Reads whether the catalog is usable and, when it is, searches again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let ready = self.deps.backend.ready();
        self.ready_task = Some(cx.spawn(async move |this, cx| {
            let readiness = spawn_kube(cx, ready).await;
            this.update(cx, |this, cx| {
                let readiness = readiness
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                this.phase = match readiness {
                    Ok(ChartsReadiness::NoHelm) => Phase::NoHelm,
                    Ok(ChartsReadiness::NoRepos) => Phase::NoRepos,
                    Ok(ChartsReadiness::Ready(repos)) => Phase::Ready { repos: repos.len() },
                    Err(why) => Phase::Failed(why),
                };
                if matches!(this.phase, Phase::Ready { .. }) {
                    this.search_now(cx);
                } else {
                    this.clear_results();
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn clear_results(&mut self) {
        self.charts = Arc::default();
        self.cards.clear();
        self.selected = None;
        self.detail = None;
        self.searching = false;
    }

    /// The text in the search field.
    pub fn keyword(&self, cx: &App) -> String {
        self.query.read(cx).value().to_string()
    }

    fn on_query(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => self.search_debounced(cx),
            InputEvent::PressEnter { .. } => self.search_now(cx),
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// Searches after typing pauses. The text field itself never waits.
    fn search_debounced(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Ready { .. }) {
            return;
        }
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| this.search_now(cx)).ok();
        }));
    }

    /// Searches for the text in the field now, replacing a search in flight.
    pub fn search_now(&mut self, cx: &mut Context<Self>) {
        let keyword = self.keyword(cx);
        let search = self.deps.backend.search(keyword);
        self.searching = true;
        self.problem = None;
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let found = spawn_kube(cx, search).await;
            this.update(cx, |this, cx| {
                this.searching = false;
                match found
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    Ok(charts) => this.show(charts, cx),
                    Err(why) => this.problem = Some(why),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Shows `charts`, keeping the selected chart selected when it is still there.
    fn show(&mut self, charts: Arc<Vec<ChartSummary>>, cx: &mut Context<Self>) {
        let keep = self
            .selected_chart()
            .map(|c| (c.repo.clone(), c.name.clone()));
        self.cards = charts.iter().map(Card::of).collect();
        self.charts = charts;
        self.selected = keep.and_then(|(repo, name)| {
            self.charts
                .iter()
                .position(|c| c.repo == repo && c.name == name)
        });
        if self.selected.is_none() {
            self.detail = None;
        }
        if self.cards.is_empty() {
            self.list_scroll = UniformListScrollHandle::new();
        }
        let _ = cx;
    }

    /// The selected chart.
    pub fn selected_chart(&self) -> Option<&ChartSummary> {
        self.selected.and_then(|i| self.charts.get(i))
    }

    /// Selects the `index`th chart: its README loads, off the UI thread.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(chart) = self.charts.get(index) else {
            return;
        };
        if self.selected == Some(index) {
            return;
        }
        self.selected = Some(index);
        self.detail = Some(Detail::new(chart, cx));
        self.load_tab(Tab::Readme, cx);
        cx.notify();
    }

    /// Opens a detail tab; its text loads the first time.
    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if let Some(detail) = &mut self.detail {
            detail.tab = tab;
        }
        self.load_tab(tab, cx);
        cx.notify();
    }

    /// Switches the selected chart to `version`: README and values are read again for it.
    pub fn set_version(&mut self, version: &str, cx: &mut Context<Self>) {
        let Some(chart) = self.selected.and_then(|i| self.charts.get(i)) else {
            return;
        };
        let Some(detail) = &mut self.detail else {
            return;
        };
        if detail.version == version || !chart.versions.iter().any(|v| v.version == version) {
            return;
        }
        detail.version = version.to_owned();
        detail.reset();
        let tab = detail.tab;
        self.load_tab(tab, cx);
        cx.notify();
    }

    fn load_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let Some(chart) = self.selected.and_then(|i| self.charts.get(i)).cloned() else {
            return;
        };
        let backend = self.deps.backend.clone();
        if let Some(detail) = &mut self.detail {
            detail.load(&chart, tab, backend.as_ref(), cx);
        }
    }

    /// Hands the selected chart and version to the install tab: `helm::InstallChart`.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let (Some(chart), Some(detail)) = (self.selected_chart(), &self.detail) else {
            return;
        };
        let Some(repo) = chart.repo.clone() else {
            return;
        };
        let command = Command::HelmInstallChart {
            repo,
            name: chart.name.clone(),
            version: Some(detail.version.clone()),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Opens the repositories page: `helm::ShowRepos`.
    pub fn open_repos(&mut self, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(Command::HelmShowRepos, cx);
    }

    /// Adds one of the default repositories: `helm::RepoAdd`.
    pub fn add_repo(&mut self, name: &str, url: &str, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(
            Command::HelmRepoAdd {
                name: name.to_owned(),
                url: url.to_owned(),
                username: None,
                insecure_skip_tls_verify: false,
                ca_file: None,
            },
            cx,
        );
    }

    /// What the page is doing.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The cards of the last search.
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    /// The open detail.
    pub fn detail(&self) -> Option<&Detail> {
        self.detail.as_ref()
    }

    /// A search is running.
    pub fn is_searching(&self) -> bool {
        self.searching
    }

    /// The last search's failure.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}
