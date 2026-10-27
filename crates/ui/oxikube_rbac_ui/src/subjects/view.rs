//! [`SubjectsView`]: the entity of the page, its state and its actions.

use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, WeakEntity, Window,
};
use oxikube_app::rbac_subjects::{
    Inherit, MatrixColumn, PolicyHandle, PolicyStatus, Provenance, SubjectFilter, SubjectMatrix,
};
use oxikube_app::store::ResourceStore;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{GrantScope, Subject};
use oxikube_runtime::spawn_kube;
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_ui::table::{TableHandle, TableOptions};
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::backend::SubjectsBackend;
use super::grid::MatrixDelegate;
use super::list::ListDelegate;
use super::model::{Page, Phase};

/// How long the page waits after a change before reading the index again, so a burst of binding
/// events (a Helm install writes dozens) costs one pass.
pub const SETTLE: Duration = Duration::from_millis(150);

/// What the page needs from outside. Everything is handed in, so a test builds the view over
/// fixtures and the app builds it over its store and bus.
#[derive(Clone)]
pub struct SubjectsDeps {
    /// Where the policy comes from.
    pub backend: Rc<dyn SubjectsBackend>,
    /// Where the page's commands go (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
}

/// The RBAC subject pages of one cluster. See the [module docs](super).
pub struct SubjectsView {
    pub(crate) cluster: ClusterId,
    pub(crate) deps: SubjectsDeps,
    pub(crate) workspace: Option<WeakEntity<Workspace>>,
    pub(crate) focus: FocusHandle,
    pub(crate) page: Page,
    pub(crate) phase: Phase,
    pub(crate) status: PolicyStatus,
    pub(crate) policy: Option<PolicyHandle>,
    pub(crate) list: TableHandle<ListDelegate>,
    pub(crate) grid: TableHandle<MatrixDelegate>,
    pub(crate) filter_input: Entity<InputState>,
    pub(crate) filter: SubjectFilter,
    pub(crate) editing: bool,
    /// The subject name as asked for (`:rbac <name>`).
    pub(crate) asked: String,
    /// The subject the matrix is for, once the policy resolved the name.
    pub(crate) subject: Option<Subject>,
    /// The scope the user picked; `None` picks one: cluster-wide when the subject has grants
    /// there, else the first namespace.
    pub(crate) picked_scope: Option<GrantScope>,
    pub(crate) scope: GrantScope,
    pub(crate) scopes: Vec<GrantScope>,
    pub(crate) inherit: Inherit,
    pub(crate) matrix: Option<Arc<SubjectMatrix>>,
    pub(crate) column: MatrixColumn,
    /// The bindings and roles behind the selected cell.
    pub(crate) why: Vec<Provenance>,
    pub(super) generation: u64,
    pub(super) read_task: Option<Task<()>>,
    pub(super) why_task: Option<Task<()>>,
    feed_task: Option<Task<()>>,
    /// The store the feeds follow, to notice a reconnect.
    followed: Option<ResourceStore>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<oxikube_workspace::ItemEvent> for SubjectsView {}

impl Focusable for SubjectsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SubjectsView {
    /// A page over `deps`, showing `page` (for [`Page::Subject`], `asked` names the subject).
    /// The policy is followed at once.
    pub fn new(
        deps: SubjectsDeps,
        workspace: Option<WeakEntity<Workspace>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = TableOptions {
            sortable: false,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: false,
        };
        let list = TableHandle::with_options(ListDelegate::default(), options, window, cx);
        let grid = TableHandle::with_options(MatrixDelegate::default(), options, window, cx);
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let mut this = Self {
            cluster: deps.backend.cluster().clone(),
            deps,
            workspace,
            focus: cx.focus_handle(),
            page: Page::Users,
            phase: Phase::Loading { pending: 4 },
            status: PolicyStatus::default(),
            policy: None,
            list,
            grid,
            filter_input,
            filter: SubjectFilter::default(),
            editing: false,
            asked: String::new(),
            subject: None,
            picked_scope: None,
            scope: GrantScope::Cluster,
            scopes: vec![GrantScope::Cluster],
            inherit: Inherit::Direct,
            matrix: None,
            column: MatrixColumn::Verb(Verb::Get),
            why: Vec::new(),
            generation: 0,
            read_task: None,
            why_task: None,
            feed_task: None,
            followed: None,
            _subscriptions: Vec::new(),
        };
        this.wire(window, cx);
        this.follow(cx);
        this
    }

    fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.filter_input.clone();
        let typing = cx.subscribe_in(
            &filter,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.set_filter_text(&text, cx);
                }
                InputEvent::PressEnter { .. } => this.focus_rows(window, cx),
                InputEvent::Focus => this.set_editing(true, cx),
                InputEvent::Blur => this.set_editing(false, cx),
            },
        );
        self._subscriptions.push(typing);
        // Clicking a header or a row focuses the library's tables; the keys live on the page.
        for handle in [self.list.focus_handle(cx), self.grid.focus_handle(cx)] {
            let refocus = cx.on_focus(&handle, window, |this, window, cx| {
                window.focus(&this.focus, cx);
            });
            self._subscriptions.push(refocus);
        }
        let weak = cx.entity().downgrade();
        let list_events = self.list.on_event(cx, {
            let weak = weak.clone();
            move |event, cx| {
                weak.update(cx, |this, cx| this.on_list_event(event, cx))
                    .ok();
            }
        });
        let grid_events = self.grid.on_event(cx, move |event, cx| {
            weak.update(cx, |this, cx| this.on_grid_event(event, cx))
                .ok();
        });
        self._subscriptions.extend([list_events, grid_events]);
    }

    /// Starts reading the policy and re-reads it whenever the feeds change.
    fn follow(&mut self, cx: &mut Context<Self>) {
        self.feed_task = None;
        self.followed = None;
        let Some(watch) = self.deps.backend.follow() else {
            self.policy = None;
            self.phase = Phase::Unavailable("The cluster is not connected.".to_owned());
            self.list
                .update_quiet(cx, |d| d.set_empty_note("The cluster is not connected."));
            return;
        };
        self.policy = Some(watch.handle);
        self.followed = watch.store;
        if let Some(mut feed) = watch.feed {
            let (tx, mut changes) = unbounded::<()>();
            // The feeds run on the runtime; the page learns of each change through the channel.
            let pump = spawn_kube(cx, async move {
                while feed.changed().await {
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
            });
            self.feed_task = Some(cx.spawn(async move |this, cx| {
                let _pump = pump;
                while changes.next().await.is_some() {
                    // One read for the burst: wait for it to settle, then drop what piled up.
                    cx.background_executor().timer(SETTLE).await;
                    while matches!(futures::poll!(changes.next()), Poll::Ready(Some(()))) {}
                    if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        self.refresh(cx);
    }

    /// Follows the policy again when the page has none (it opened while disconnected) or the
    /// cluster's store was replaced (a reconnect). Called each time the page is asked to show.
    pub(super) fn refollow_if_stale(&mut self, cx: &mut Context<Self>) {
        let now = self.deps.backend.current_store();
        let stale = match (&self.followed, &now) {
            (Some(old), Some(new)) => !old.is_same(new),
            (None, _) => self.policy.is_none() || now.is_some(),
            (Some(_), None) => false,
        };
        if stale {
            self.follow(cx);
        }
    }

    pub(super) fn set_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
        if self.editing != editing {
            self.editing = editing;
            cx.notify();
        }
    }

    // --- pages --------------------------------------------------------------------------------

    /// Shows the Users page.
    pub fn show_users(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_list(Page::Users, window, cx);
    }

    /// Shows the Groups page.
    pub fn show_groups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_list(Page::Groups, window, cx);
    }

    fn show_list(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.matrix = None;
        self.why.clear();
        self.clear_filter(window, cx);
        self.refresh(cx);
        cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
        cx.notify();
    }

    /// Shows what the policy grants the subject `name` (a user, `system:serviceaccount:<ns>:<name>`
    /// or a group).
    pub fn show_subject(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Subject;
        self.asked = name.trim().to_owned();
        self.subject = None;
        self.picked_scope = None;
        self.matrix = None;
        self.why.clear();
        self.clear_filter(window, cx);
        self.refresh(cx);
        cx.emit(oxikube_workspace::ItemEvent::UpdateTab);
        cx.notify();
    }

    /// Goes back from a matrix to the Users or Groups page it came from.
    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Subject {
            return;
        }
        let page = match self.subject {
            Some(Subject::Group(_)) => Page::Groups,
            _ => Page::Users,
        };
        self.show_list(page, window, cx);
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = SubjectFilter::default();
        self.filter_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    // --- filter, selection ---------------------------------------------------------------------

    /// Applies the filter text (typed, or put there by the page).
    pub fn set_filter_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.filter = SubjectFilter::parse(text);
        let filter = self.filter.clone();
        match self.page {
            Page::Users | Page::Groups => {
                self.list.update_quiet(cx, |d| d.reshape(&filter));
                self.list.refresh(cx);
            }
            Page::Subject => {
                self.grid.update_quiet(cx, |d| d.reshape(&filter));
                self.grid.refresh(cx);
                self.read_why(cx);
            }
        }
        cx.notify();
    }

    /// The tab title.
    pub(crate) fn title(&self) -> SharedString {
        match self.page {
            Page::Users => "RBAC Users".into(),
            Page::Groups => "RBAC Groups".into(),
            Page::Subject => format!("RBAC {}", self.asked).into(),
        }
    }

    /// The page being shown.
    pub fn page(&self) -> Page {
        self.page
    }

    /// The list table.
    pub fn list(&self) -> &TableHandle<ListDelegate> {
        &self.list
    }

    /// The matrix table.
    pub fn grid(&self) -> &TableHandle<MatrixDelegate> {
        &self.grid
    }

    /// The subject whose matrix is shown, once resolved.
    pub fn subject(&self) -> Option<&Subject> {
        self.subject.as_ref()
    }

    /// The scope of the matrix.
    pub fn scope(&self) -> &GrantScope {
        &self.scope
    }

    /// The bindings and roles behind the selected cell.
    pub fn provenance(&self) -> &[Provenance] {
        &self.why
    }

    /// The verb column the detail is for.
    pub fn column(&self) -> MatrixColumn {
        self.column
    }

    /// The matrix, once computed.
    pub fn matrix(&self) -> Option<&Arc<SubjectMatrix>> {
        self.matrix.as_ref()
    }

    /// Where loading stands.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }
}
