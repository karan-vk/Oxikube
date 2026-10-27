//! The `rbac::*` commands on the `CommandBus` and the window's controller.
//!
//! `rbac::OpenUsers` (`:usr`), `rbac::OpenGroups` (`:grp`) and `rbac::ShowSubject`
//! (`:rbac <subject>`) open the page of a cluster in its tab; `rbac::CopyMatrix` copies the open
//! matrix. None changes a cluster, so none has a `MutationGuard` tier and all run on a read-only
//! cluster; each is declared in `oxikube_domain::command`, so each has an MCP tool stub
//! (`k8s.rbac_open_users`, ...). [`register_commands`] installs handlers that push an
//! [`RbacRequest`] into the window's [`SubjectsRequestSink`]; [`SubjectsViews`] applies it on the
//! UI thread. The sidebar's Users and Groups entries and the palette send the same commands.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Context, Entity, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_workspace::{ClusterTabs, CommandDispatcher, ItemHandle, OpenOptions, Workspace};

use super::backend::SubjectsBackend;
use super::view::{SubjectsDeps, SubjectsView};

/// The commands this module handles.
pub const RBAC_COMMANDS: [CommandId; 4] = [
    CommandId::RBAC_OPEN_USERS,
    CommandId::RBAC_OPEN_GROUPS,
    CommandId::RBAC_SHOW_SUBJECT,
    CommandId::RBAC_COPY_MATRIX,
];

/// What an `rbac::*` command does to the cluster's subject page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// `rbac::OpenUsers`.
    Users,
    /// `rbac::OpenGroups`.
    Groups,
    /// `rbac::ShowSubject`: the subject as typed.
    Subject(String),
    /// `rbac::CopyMatrix`.
    CopyMatrix,
}

/// One `rbac::*` command, for the window's [`SubjectsViews`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RbacRequest {
    /// The cluster whose page it is about.
    pub cluster: ClusterId,
    /// What to do.
    pub change: Change,
}

impl RbacRequest {
    /// The request `command` makes; `None` for a command that is not an `rbac::*` one.
    pub fn of(command: &Command) -> Option<Self> {
        let (cluster, change) = match command {
            Command::RbacOpenUsers { cluster } => (cluster, Change::Users),
            Command::RbacOpenGroups { cluster } => (cluster, Change::Groups),
            Command::RbacShowSubject { cluster, subject } => {
                (cluster, Change::Subject(subject.clone()))
            }
            Command::RbacCopyMatrix { cluster } => (cluster, Change::CopyMatrix),
            _ => return None,
        };
        Some(Self {
            cluster: cluster.clone(),
            change,
        })
    }
}

/// A handle on a window's request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct SubjectsRequestSink {
    tx: UnboundedSender<RbacRequest>,
}

impl SubjectsRequestSink {
    /// A sink and the receiver [`SubjectsViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<RbacRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. `false` when the window is gone.
    pub fn send(&self, request: RbacRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }
}

/// Registers [`RBAC_COMMANDS`] on `registry` with handlers that push into `sink`. The binary
/// installs it as `oxikube_rbac_ui`.
///
/// # Errors
///
/// The first [`RegisterError`] (a duplicate registration is a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: SubjectsRequestSink,
) -> Result<(), RegisterError> {
    for id in RBAC_COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = RbacRequest::of(&command)
                    .ok_or_else(|| OxiError::validation("not an rbac command"))?;
                if sink.send(request) {
                    Ok(CommandOutput::none())
                } else {
                    Err(OxiError::internal("the window with the RBAC pages is gone"))
                }
            }
        })?;
    }
    Ok(())
}

/// Where a cluster's subject page lives: the workspace of its tab in this window. The app's is
/// the window's [`ClusterTabs`] (`WeakEntity<ClusterTabs>` implements it); a test hands a
/// workspace.
pub trait SubjectsHost: 'static {
    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (its page is opening).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);
}

impl SubjectsHost for WeakEntity<ClusterTabs> {
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>> {
        let tabs = self.upgrade()?;
        let tab = tabs.read(cx).tab(cluster)?;
        Some(tab.read(cx).workspace().clone())
    }

    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App) {
        if let Some(tabs) = self.upgrade() {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(cluster, window, cx);
            });
        }
    }
}

/// Makes the backend of one cluster's page.
pub type Backends = Rc<dyn Fn(&ClusterId) -> Rc<dyn SubjectsBackend>>;

/// What [`SubjectsViews`] needs.
#[derive(Clone)]
pub struct SubjectsViewsDeps {
    /// The backend of each cluster.
    pub backends: Backends,
    /// Where the pages' commands go (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Where the pages of a cluster live (the window's cluster tabs).
    pub host: Rc<dyn SubjectsHost>,
}

/// Opens and drives the subject pages of one window.
pub struct SubjectsViews {
    deps: SubjectsViewsDeps,
    _requests: Task<()>,
}

impl SubjectsViews {
    /// Starts the controller over `requests` (the receiver of the window's
    /// [`SubjectsRequestSink`]). Keep the entity for the life of the window.
    pub fn start(
        deps: SubjectsViewsDeps,
        mut requests: UnboundedReceiver<RbacRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let pump = cx.spawn_in(window, async move |this, cx| {
                while let Some(request) = requests.next().await {
                    let applied = this.update_in(cx, |views: &mut Self, window, cx| {
                        views.apply(request, window, cx);
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            });
            Self {
                deps,
                _requests: pump,
            }
        })
    }

    /// Applies `request`: the `Open` / `Show` commands show (or open) the cluster's page; the
    /// copy acts on the open page and does nothing when the cluster has none here.
    pub fn apply(&mut self, request: RbacRequest, window: &mut Window, cx: &mut Context<Self>) {
        let RbacRequest { cluster, change } = request;
        let Some(workspace) = self.deps.host.workspace(&cluster, cx) else {
            tracing::debug!(%cluster, "rbac: the cluster has no tab in this window");
            return;
        };
        if change == Change::CopyMatrix {
            match SubjectsView::find(&workspace, cx) {
                Some(view) => view.update(cx, |view, cx| view.copy_matrix(cx)),
                None => tracing::debug!(%cluster, "rbac: no subject page open for the cluster"),
            }
            return;
        }
        // The tab first: showing it focuses the tab itself, and the page must end up with the
        // focus so its keys work at once.
        self.deps.host.show(&cluster, window, cx);
        let view = self.open(&workspace, &cluster, window, cx);
        view.update(cx, |view, cx| {
            match change {
                Change::Users => view.show_users(window, cx),
                Change::Groups => view.show_groups(window, cx),
                Change::Subject(name) => view.show_subject(&name, window, cx),
                Change::CopyMatrix => {}
            }
            view.focus_rows(window, cx);
            window.focus(&view.focus, cx);
        });
    }

    fn open(
        &self,
        workspace: &Entity<Workspace>,
        cluster: &ClusterId,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SubjectsView> {
        if let Some(open) = SubjectsView::find(workspace, cx) {
            let id = open.entity_id();
            workspace.update(cx, |ws, cx| ws.activate_item(id, true, window, cx));
            return open;
        }
        let deps = SubjectsDeps {
            backend: (self.deps.backends)(cluster),
            dispatcher: self.deps.dispatcher.clone(),
        };
        let weak = workspace.downgrade();
        let view = cx.new(|cx| SubjectsView::new(deps, Some(weak), window, cx));
        let item: Box<dyn ItemHandle> = Box::new(view.clone());
        let options = OpenOptions {
            focus: true,
            reuse_existing: true,
            ..OpenOptions::default()
        };
        workspace.update(cx, |ws, cx| ws.open_item_with(item, options, window, cx));
        view
    }
}
