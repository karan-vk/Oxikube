//! `pod::BrowseFiles` on the `CommandBus` (E20-S07), and the controller that opens the file
//! browser it asks for.
//!
//! The command reads nothing itself: [`register_commands`] installs a handler that pushes a
//! [`FilesRequest`] into the window's [`FilesCommandSink`], and [`FilesViews`] opens the pod's
//! [`FilesView`](super::FilesView) in its cluster tab on the UI thread. The transfers
//! (`pod::DownloadFile`, `pod::UploadFile`) are `oxikube_app::files`'s handlers.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Context, Entity, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_workspace::{ClusterTabs, Workspace};

use super::view::{FilesView, FilesViewDeps};

/// One `pod::BrowseFiles` command, for the window's [`FilesViews`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesRequest {
    /// The pod.
    pub pod: ResourceRef,
    /// The container to open; `None` is the pod's default.
    pub container: Option<String>,
    /// The directory to open; `None` is the last one used for the pod, else `/`.
    pub path: Option<String>,
}

impl FilesRequest {
    /// The request `command` makes; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        let Command::PodBrowseFiles {
            target,
            container,
            path,
        } = command
        else {
            return None;
        };
        Some(Self {
            pod: target.clone(),
            container: container.clone(),
            path: path.clone(),
        })
    }
}

/// A handle on a window's file browser request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct FilesCommandSink {
    tx: UnboundedSender<FilesRequest>,
}

impl FilesCommandSink {
    /// A sink and the receiver [`FilesViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<FilesRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Queues `request`. `false` when the window is gone.
    pub fn send(&self, request: FilesRequest) -> bool {
        self.tx.unbounded_send(request).is_ok()
    }
}

/// Registers `pod::BrowseFiles` on `registry` with a handler that pushes into `sink`. The binary
/// installs it as `oxikube_resources_ui::files`.
///
/// # Errors
///
/// A [`RegisterError`] when the command is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: FilesCommandSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::POD_BROWSE_FILES)
        .ok_or(RegisterError::Undeclared(CommandId::POD_BROWSE_FILES))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let request = FilesRequest::of(&command)
                .ok_or_else(|| OxiError::validation("not a browse files command"))?;
            if sink.send(request) {
                Ok(CommandOutput::none())
            } else {
                Err(OxiError::internal(
                    "the window with the file browsers is gone",
                ))
            }
        }
    })
}

/// Where a cluster's file browsers live: the workspace of its tab in this window. The app's is the
/// window's [`ClusterTabs`] (`WeakEntity<ClusterTabs>` implements it); a test hands a workspace.
pub trait FilesHost: 'static {
    /// The workspace of `cluster`'s tab, `None` when it has no tab here.
    fn workspace(&self, cluster: &ClusterId, cx: &App) -> Option<Entity<Workspace>>;

    /// Shows `cluster`'s tab (its file browser is opening).
    fn show(&self, cluster: &ClusterId, window: &mut Window, cx: &mut App);
}

impl FilesHost for WeakEntity<ClusterTabs> {
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

/// What [`FilesViews`] needs.
#[derive(Clone)]
pub struct FilesViewsDeps {
    /// What every file browser is built over.
    pub view: FilesViewDeps,
    /// Where the browsers of a cluster live (the window's cluster tabs).
    pub host: Rc<dyn FilesHost>,
}

/// Opens the file browsers of one window.
pub struct FilesViews {
    deps: FilesViewsDeps,
    _requests: Task<()>,
}

impl FilesViews {
    /// Starts the controller over `requests` (the receiver of the window's [`FilesCommandSink`]).
    /// Keep the entity for the life of the window.
    pub fn start(
        deps: FilesViewsDeps,
        mut requests: UnboundedReceiver<FilesRequest>,
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

    /// Shows the pod's file browser in its cluster's tab: the open one, or a new tab.
    pub fn apply(&mut self, request: FilesRequest, window: &mut Window, cx: &mut Context<Self>) {
        let cluster = request.pod.cluster.clone();
        let Some(workspace) = self.deps.host.workspace(&cluster, cx) else {
            tracing::debug!(%cluster, "files: the cluster has no tab in this window");
            return;
        };
        self.deps.host.show(&cluster, window, cx);
        FilesView::open(&workspace, request, self.deps.view.clone(), window, cx);
    }
}
