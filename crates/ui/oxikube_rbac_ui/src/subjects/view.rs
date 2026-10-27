//! [`SubjectsView`]: the entity of the page, its state and its actions.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window,
};
use oxikube_app::rbac_subjects::{
    Inherit, MatrixColumn, PolicyHandle, PolicyStatus, Provenance, SubjectFilter, SubjectList,
    SubjectMatrix, provenance, resolve_subject, scopes_of, subject_matrix, subject_name, summaries,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{GrantScope, Subject};
use oxikube_runtime::{notify_coalesced, spawn_kube};
use oxikube_ui::input::{InputEvent, InputState};
use oxikube_ui::table::{RowClick, TableEvent, TableHandle, TableOptions};
use oxikube_workspace::{CommandDispatcher, Toast, Workspace};

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
    generation: u64,
    read_task: Option<Task<()>>,
    why_task: Option<Task<()>>,
    feed_task: Option<Task<()>>,
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
        let Some(watch) = self.deps.backend.follow() else {
            self.policy = None;
            self.phase = Phase::Unavailable("The cluster is not connected.".to_owned());
            self.list
                .update_quiet(cx, |d| d.set_empty_note("The cluster is not connected."));
            return;
        };
        self.policy = Some(watch.handle);
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
                    while futures::poll!(changes.next()).is_ready() {}
                    if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        self.refresh(cx);
    }

    fn set_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
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

    // --- reading the policy -------------------------------------------------------------------

    /// Reads the index again for the page that is shown, on the background executor. A request
    /// while one is running replaces it (the newest wins).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.policy.clone() else {
            return;
        };
        self.status = handle.status();
        self.phase = if self.status.pending > 0 {
            Phase::Loading {
                pending: self.status.pending,
            }
        } else {
            Phase::Ready
        };
        self.generation += 1;
        let generation = self.generation;
        match self.page {
            Page::Users | Page::Groups => {
                let list = if self.page == Page::Users {
                    SubjectList::Users
                } else {
                    SubjectList::Groups
                };
                self.read_task = Some(cx.spawn(async move |this, cx| {
                    let rows = cx
                        .background_executor()
                        .spawn(async move { handle.read(|index| summaries(index, list)) })
                        .await;
                    this.update(cx, |this, cx| this.listed(generation, rows, cx))
                        .ok();
                }));
            }
            Page::Subject => {
                let asked = self.asked.clone();
                let picked = self.picked_scope.clone();
                let inherit = self.inherit;
                self.read_task = Some(cx.spawn(async move |this, cx| {
                    let computed = cx
                        .background_executor()
                        .spawn(async move { compute(&handle, &asked, picked, inherit) })
                        .await;
                    this.update(cx, |this, cx| this.computed(generation, computed, cx))
                        .ok();
                }));
            }
        }
        cx.notify();
    }

    fn listed(
        &mut self,
        generation: u64,
        rows: Vec<oxikube_app::rbac_subjects::SubjectSummary>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation || self.page == Page::Subject {
            return;
        }
        let filter = self.filter.clone();
        let note = match (&self.phase, self.page) {
            (Phase::Loading { pending }, _) => {
                format!("Reading roles and bindings ({pending} of 4 lists pending)...")
            }
            (_, Page::Groups) => "No binding names a group.".to_owned(),
            _ => "No binding names a user or ServiceAccount.".to_owned(),
        };
        self.list.update_quiet(cx, |d| {
            d.set_rows(rows, &filter);
            d.set_empty_note(note);
        });
        self.list.refresh(cx);
        notify_coalesced(cx);
    }

    fn computed(&mut self, generation: u64, computed: Computed, cx: &mut Context<Self>) {
        if generation != self.generation || self.page != Page::Subject {
            return;
        }
        let Computed {
            subject,
            scopes,
            scope,
            matrix,
        } = computed;
        let note = self.empty_matrix_note(&subject, &matrix);
        let matrix = Arc::new(matrix);
        let selected_line = self.grid.read(cx, |d| {
            d.selected_row().and_then(|row| d.line(row).cloned())
        });
        let filter = self.filter.clone();
        let column = self.column;
        self.grid.update_quiet(cx, |d| {
            d.set_matrix(Some(matrix.clone()), &filter);
            d.set_note(note);
            d.set_column(Some(column));
            // The selection follows its line (the same group and resource) through a refresh.
            let row = selected_line.and_then(|old| {
                (0..d.len()).find(|&row| {
                    d.line(row)
                        .is_some_and(|l| l.api_group == old.api_group && l.resource == old.resource)
                })
            });
            d.select_row(row);
        });
        self.subject = Some(subject);
        self.scopes = scopes;
        self.scope = scope;
        self.matrix = Some(matrix);
        self.grid.refresh(cx);
        self.read_why(cx);
        notify_coalesced(cx);
    }

    fn empty_matrix_note(&self, subject: &Subject, matrix: &SubjectMatrix) -> String {
        if !matrix.is_empty() {
            return String::new();
        }
        if self.status.pending > 0 {
            return format!(
                "Reading roles and bindings ({} of 4 lists pending)...",
                self.status.pending
            );
        }
        let known = self.policy.as_ref().is_some_and(|policy| {
            policy.read(|index| index.subjects().iter().any(|s| same_identity(s, subject)))
        });
        if known {
            format!(
                "No rules reach {} {}. Its bindings may name roles that do not exist.",
                subject.kind(),
                subject_name(subject)
            )
        } else {
            format!(
                "No bindings found for \"{}\". Check the spelling, or pick a subject from Users or \
                 Groups.",
                self.asked
            )
        }
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

    fn on_list_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(RowClick { row, count, .. }) = event {
            self.select_row(*row, cx);
            if *count >= 2 {
                self.open_selected(cx);
            }
        }
    }

    fn on_grid_event(&mut self, event: &TableEvent, cx: &mut Context<Self>) {
        if let TableEvent::RowClicked(RowClick { row, .. }) = event {
            self.select_row(*row, cx);
        }
    }

    /// Selects row `row` of the shown table.
    pub fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.page == Page::Subject {
            self.grid.update_quiet(cx, |d| d.select_row(Some(row)));
            self.grid.refresh(cx);
            self.grid.reveal_row(row, cx);
            self.read_why(cx);
        } else {
            let Some(subject) = self.list.read(cx, |d| d.subject(row).cloned()) else {
                return;
            };
            self.list.update_quiet(cx, |d| d.select(Some(subject)));
            self.list.refresh(cx);
            self.list.reveal_row(row, cx);
        }
        cx.notify();
    }

    /// Moves the selection `delta` rows (the first row when none is selected).
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (count, current) = if self.page == Page::Subject {
            self.grid.read(cx, |d| (d.len(), d.selected_row()))
        } else {
            self.list.read(cx, |d| {
                (d.len(), d.selected().and_then(|s| d.position_of(s)))
            })
        };
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
        let count = if self.page == Page::Subject {
            self.grid.read(cx, |d| d.len())
        } else {
            self.list.read(cx, |d| d.len())
        };
        if count > 0 {
            self.select_row(if last { count - 1 } else { 0 }, cx);
        }
    }

    /// Opens the matrix of the selected list row: the command `rbac::ShowSubject`, the same one
    /// `:rbac`, the palette and an agent run.
    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        if self.page == Page::Subject {
            return;
        }
        let Some(subject) = self.list.read(cx, |d| d.selected().cloned()) else {
            return;
        };
        self.deps.dispatcher.dispatch(
            Command::RbacShowSubject {
                cluster: self.cluster.clone(),
                subject: subject_name(&subject).to_string(),
            },
            cx,
        );
    }

    pub(super) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn focus_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.set_editing(false, cx);
    }

    // --- the matrix: scope, groups, columns, provenance ----------------------------------------

    /// Shows the matrix of `scope`.
    pub fn set_scope(&mut self, scope: GrantScope, cx: &mut Context<Self>) {
        if self.page != Page::Subject || scope == self.scope {
            return;
        }
        self.picked_scope = Some(scope);
        self.refresh(cx);
    }

    /// The next scope: cluster-wide, then each namespace, round.
    pub fn next_scope(&mut self, cx: &mut Context<Self>) {
        if self.scopes.len() < 2 {
            return;
        }
        let at = self
            .scopes
            .iter()
            .position(|s| *s == self.scope)
            .unwrap_or(0);
        let next = self.scopes[(at + 1) % self.scopes.len()].clone();
        self.set_scope(next, cx);
    }

    /// Includes the groups the API server adds on its own, or leaves them out.
    pub fn toggle_implicit_groups(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Subject {
            return;
        }
        self.inherit = match self.inherit {
            Inherit::Direct => Inherit::ImplicitGroups,
            Inherit::ImplicitGroups => Inherit::Direct,
        };
        self.refresh(cx);
    }

    /// Picks the verb column whose granting bindings the detail lists.
    pub fn set_column(&mut self, column: MatrixColumn, cx: &mut Context<Self>) {
        self.column = column;
        self.grid.update_quiet(cx, |d| d.set_column(Some(column)));
        self.grid.refresh(cx);
        self.read_why(cx);
        cx.notify();
    }

    /// Moves the picked column one step, stopping at the ends.
    pub fn step_column(&mut self, forward: bool, cx: &mut Context<Self>) {
        let all: Vec<MatrixColumn> = MatrixColumn::all().collect();
        let at = all.iter().position(|c| *c == self.column).unwrap_or(0);
        let next = if forward {
            (at + 1).min(all.len() - 1)
        } else {
            at.saturating_sub(1)
        };
        self.set_column(all[next], cx);
    }

    /// Reads the bindings behind the selected cell (off the UI thread).
    fn read_why(&mut self, cx: &mut Context<Self>) {
        self.why_task = None;
        let (Some(policy), Some(subject)) = (self.policy.clone(), self.subject.clone()) else {
            self.why.clear();
            return;
        };
        let Some(line) = self
            .grid
            .read(cx, |d| d.selected_row().and_then(|r| d.line(r).cloned()))
        else {
            self.why.clear();
            return;
        };
        let (scope, inherit, column) = (self.scope.clone(), self.inherit, self.column);
        let generation = self.generation;
        self.why_task = Some(cx.spawn(async move |this, cx| {
            let why = cx
                .background_executor()
                .spawn(async move {
                    policy.read(|index| provenance(index, &subject, &scope, inherit, &line, column))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.why = why;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Copies the matrix as tab-separated text (`rbac::CopyMatrix`).
    pub fn copy_matrix(&mut self, cx: &mut Context<Self>) {
        let Some(matrix) = self.matrix.clone().filter(|m| !m.is_empty()) else {
            self.toast(Toast::info("There is no matrix to copy."), cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(matrix.to_tsv()));
        self.toast(
            Toast::info(format!("Copied {} rows as text.", matrix.lines.len())),
            cx,
        );
    }

    /// Asks the bus to copy the matrix: the button and the key send the command.
    pub(super) fn request_copy(&mut self, cx: &mut Context<Self>) {
        self.deps.dispatcher.dispatch(
            Command::RbacCopyMatrix {
                cluster: self.cluster.clone(),
            },
            cx,
        );
    }

    fn toast(&self, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = &self.workspace {
            workspace
                .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                .ok();
        }
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

/// What the background pass hands back for a subject page.
struct Computed {
    subject: Subject,
    scopes: Vec<GrantScope>,
    scope: GrantScope,
    matrix: SubjectMatrix,
}

/// Resolves the typed name, lists its scopes and projects its matrix.
fn compute(
    policy: &PolicyHandle,
    asked: &str,
    picked: Option<GrantScope>,
    inherit: Inherit,
) -> Computed {
    policy.read(|index| {
        let subject = resolve_subject(index, asked);
        let scopes = scopes_of(index, &subject, inherit);
        let scope = picked.unwrap_or_else(|| {
            // Cluster-wide when the subject has grants there; otherwise the first namespace.
            let cluster = subject_matrix(index, &subject, &GrantScope::Cluster, inherit);
            match scopes.get(1) {
                Some(first) if cluster.grants == 0 => first.clone(),
                _ => GrantScope::Cluster,
            }
        });
        let matrix = subject_matrix(index, &subject, &scope, inherit);
        Computed {
            subject,
            scopes,
            scope,
            matrix,
        }
    })
}

/// Whether two subjects are the same identity (`User system:serviceaccount:a:b` is the
/// ServiceAccount `a/b`).
fn same_identity(a: &Subject, b: &Subject) -> bool {
    let canonical = |s: &Subject| match s {
        Subject::User(name) => Subject::from_username(name),
        other => other.clone(),
    };
    canonical(a) == canonical(b)
}
