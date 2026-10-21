//! `helm::ShowRepos`: open the page in the active cluster's tab.

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, SharedString, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_helm_ui::repos::{HelmReposView, ITEM_KEY, ReposBackend, ReposDeps};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, OpenOptions};

/// A handle on the window's page-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct HelmReposRequestSink {
    tx: UnboundedSender<()>,
}

impl HelmReposRequestSink {
    /// A sink and the receiver [`HelmReposViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<()>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to show the page. Fails when the window is gone.
    pub fn send(&self) -> OxiResult<()> {
        self.tx
            .unbounded_send(())
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `helm::ShowRepos` (and its tool stub, `helm.show_repos`) on `registry`. The binary
/// installs it as `oxikube_settings_ui::helm_repos`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: HelmReposRequestSink,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::HELM_SHOW_REPOS)
        .ok_or(RegisterError::Undeclared(CommandId::HELM_SHOW_REPOS))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            if !matches!(command, Command::HelmShowRepos) {
                return Err(OxiError::validation("not a helm::ShowRepos command"));
            }
            sink.send()?;
            Ok(CommandOutput::none())
        }
    })
}

/// What opening the page needs.
#[derive(Clone)]
pub struct HelmReposViewsDeps {
    /// Where the page reads its state.
    pub backend: Rc<dyn ReposBackend>,
    /// Where the page's buttons send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
}

/// The window's Helm repositories page.
pub struct HelmReposViews;

impl HelmReposViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for the
    /// life of the window (dropping it stops it).
    pub fn start(
        deps: HelmReposViewsDeps,
        mut requests: UnboundedReceiver<()>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while requests.next().await.is_some() {
                match cx.update(|window, cx| Self::open(&deps, window, cx)) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::debug!(
                            "helm::ShowRepos: no cluster tab is open to show the page in"
                        );
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Shows the page in the active cluster's tab (the open one when there is one) and brings the
    /// tab forward. `false` when no cluster tab is open.
    pub fn open(deps: &HelmReposViewsDeps, window: &mut Window, cx: &mut App) -> bool {
        let Some(tabs) = deps.tabs.upgrade() else {
            return false;
        };
        let Some(cluster) = tabs
            .read(cx)
            .active()
            .cloned()
            .or_else(|| tabs.read(cx).clusters(cx).into_iter().next())
        else {
            return false;
        };
        let Some(workspace) = tabs
            .read(cx)
            .tab(&cluster)
            .map(|tab| tab.read(cx).workspace().clone())
        else {
            return false;
        };
        let key = SharedString::from(ITEM_KEY);
        match workspace.read(cx).find_item_by_key(&key, cx) {
            Some(item) => workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx)),
            None => {
                let page = ReposDeps {
                    backend: deps.backend.clone(),
                    dispatcher: deps.dispatcher.clone(),
                };
                let view = cx.new(|cx| HelmReposView::new(page, window, cx));
                let options = OpenOptions {
                    focus: true,
                    reuse_existing: true,
                    ..OpenOptions::default()
                };
                workspace.update(cx, |ws, cx| {
                    ws.open_item_with(Box::new(view), options, window, cx);
                });
            }
        }
        tabs.update(cx, |tabs, cx| {
            tabs.activate(&cluster, window, cx);
        });
        true
    }
}
