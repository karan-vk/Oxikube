//! `apply::Directory` and `apply::Kustomize`: open the panel in the cluster's tab.
//!
//! Both only open the panel (and its file picker, when the command names no path): they read no
//! file and change no cluster, so they are plain commands with tool stubs (`k8s.apply_directory`,
//! `k8s.apply_kustomize`). What the panel then dry-runs and applies is `resource::ApplyPlan`,
//! guarded on its own. `apply::Kustomize` needs the `kustomize` capability: the palette hides it
//! while neither `kustomize` nor `kubectl kustomize` is installed.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, SharedString, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::{ClusterTabs, OpenOptions};

use super::backend::ApplyBackend;
use super::document::EditorOpener;
use super::source::{ApplySource, SourceKind};
use super::view::{ApplyDeps, ApplyView, ITEM_KEY};

/// The commands [`register_commands`] handles.
pub const COMMANDS: [CommandId; 2] = [CommandId::APPLY_DIRECTORY, CommandId::APPLY_KUSTOMIZE];

/// A request to show the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    /// The cluster to apply to.
    pub cluster: ClusterId,
    /// How the path is read.
    pub kind: SourceKind,
    /// The file or directory to load; `None` opens the file picker.
    pub path: Option<String>,
    /// The namespace to put in the field; `None` leaves the tab's selected one.
    pub namespace: Option<String>,
}

impl ApplyRequest {
    /// The request `command` asks for; `None` for any other command.
    pub fn of(command: &Command) -> Option<Self> {
        match command {
            Command::ApplyDirectory {
                cluster,
                path,
                namespace,
            } => Some(Self {
                cluster: cluster.clone(),
                kind: SourceKind::Manifests,
                path: path.clone(),
                namespace: namespace.clone(),
            }),
            Command::ApplyKustomize {
                cluster,
                path,
                namespace,
            } => Some(Self {
                cluster: cluster.clone(),
                kind: SourceKind::Kustomize,
                path: path.clone(),
                namespace: namespace.clone(),
            }),
            _ => None,
        }
    }
}

/// A handle on the window's request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct ApplyRequestSink {
    tx: UnboundedSender<ApplyRequest>,
}

impl ApplyRequestSink {
    /// A sink and the receiver [`ApplyViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<ApplyRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to show the panel. Fails when the window is gone.
    pub fn send(&self, request: ApplyRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `apply::Directory` and `apply::Kustomize` (and their tool stubs) on `registry`. The
/// binary installs it as `oxikube_resources_ui::apply`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: ApplyRequestSink,
) -> Result<(), RegisterError> {
    for id in COMMANDS {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                let request = ApplyRequest::of(&command)
                    .ok_or_else(|| OxiError::validation("not an apply panel command"))?;
                sink.send(request)?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    Ok(())
}

/// What opening the panel needs.
#[derive(Clone)]
pub struct ApplyViewsDeps {
    /// Where the panel loads, plans and applies.
    pub backend: Rc<dyn ApplyBackend>,
    /// Opens a document in the manifest editor.
    pub editor: EditorOpener,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's apply panels.
pub struct ApplyViews;

impl ApplyViews {
    /// Opens the panel for each request of `requests`, on the UI thread. Keep the returned task
    /// for the life of the window.
    pub fn start(
        deps: ApplyViewsDeps,
        mut requests: UnboundedReceiver<ApplyRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(request) = requests.next().await {
                match cx.update(|window, cx| Self::open(&deps, request, window, cx)) {
                    Ok(true) => {}
                    Ok(false) => tracing::debug!("apply: the cluster has no tab in this window"),
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the panel in `request.cluster`'s tab (the open one when there is one), brings the tab
    /// forward and starts the request: the path loads, or the file picker opens. `false` when the
    /// cluster has no tab.
    pub fn open(
        deps: &ApplyViewsDeps,
        request: ApplyRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(tabs) = deps.tabs.upgrade() else {
            return false;
        };
        let cluster = request.cluster.clone();
        let Some(workspace) = tabs
            .read(cx)
            .tab(&cluster)
            .map(|tab| tab.read(cx).workspace().clone())
        else {
            return false;
        };
        let key = SharedString::from(ITEM_KEY);
        let view = match workspace.read(cx).find_item_by_key(&key, cx) {
            Some(item) => {
                workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
                workspace
                    .read(cx)
                    .items_of_type::<ApplyView>()
                    .into_iter()
                    .find(|view| view.entity_id() == item)
            }
            None => {
                let page = ApplyDeps {
                    backend: deps.backend.clone(),
                    cluster: cluster.clone(),
                    editor: deps.editor.clone(),
                };
                let view = cx.new(|cx| ApplyView::new(page, window, cx));
                let options = OpenOptions {
                    focus: true,
                    reuse_existing: true,
                    ..OpenOptions::default()
                };
                workspace.update(cx, |ws, cx| {
                    ws.open_item_with(Box::new(view.clone()), options, window, cx);
                });
                Some(view)
            }
        };
        tabs.update(cx, |tabs, cx| {
            tabs.activate(&cluster, window, cx);
        });
        if let Some(view) = view {
            view.update(cx, |view, cx| view.start(&request, window, cx));
        }
        true
    }
}

impl ApplyView {
    /// Starts `request`: the namespace into the field, then the path loads, or the picker opens.
    pub fn start(
        &mut self,
        request: &ApplyRequest,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(namespace) = &request.namespace {
            self.namespace.update(cx, |input, cx| {
                input.set_value(namespace.clone(), window, cx);
            });
        }
        match &request.path {
            Some(path) => {
                let source = match request.kind {
                    SourceKind::Manifests => ApplySource::manifests(path),
                    SourceKind::Kustomize => ApplySource::kustomize(path),
                };
                self.load(source, cx);
            }
            None => self.pick(request.kind, cx),
        }
    }
}
