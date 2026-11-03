//! `keymap::ShowEditor`: open the page in the active cluster's tab (in the window's own workspace
//! when no cluster tab is open); `keymap::RecordBinding`: open the key recorder on its selected
//! row (E21-S08).

use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, Entity, SharedString, Task, WeakEntity, Window};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::{ClusterTabs, CommandDispatcher, OpenOptions, Workspace};

use super::view::{ITEM_KEY, KeymapEditorDeps, KeymapEditorView};

/// What the window is asked to do with the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeymapEditorRequest {
    /// Show the page (`keymap::ShowEditor`).
    Show,
    /// Show the page and open the key recorder on its selected row (`keymap::RecordBinding`).
    Record,
}

/// A handle on the window's page-request queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct KeymapEditorRequestSink {
    tx: UnboundedSender<KeymapEditorRequest>,
}

impl KeymapEditorRequestSink {
    /// A sink and the receiver [`KeymapEditorViews::start`] drains.
    pub fn channel() -> (Self, UnboundedReceiver<KeymapEditorRequest>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }

    /// Asks the window to act on the page. Fails when the window is gone.
    pub fn send(&self, request: KeymapEditorRequest) -> OxiResult<()> {
        self.tx
            .unbounded_send(request)
            .map_err(|_| OxiError::internal("the window with the cluster tabs is gone"))
    }
}

/// Registers `keymap::ShowEditor` and `keymap::RecordBinding` (tool stubs `app.keymap_show_editor`,
/// `app.keymap_record_binding`) on `registry`, and the edits of [`KeymapEditSink`](super::KeymapEditSink)
/// (`keymap::SetBinding`, `keymap::RemoveBinding`) over `edits`. The binary installs them as
/// `oxikube_settings_ui::keymap`.
///
/// # Errors
///
/// A [`RegisterError`] when an id is already taken.
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: KeymapEditorRequestSink,
    edits: super::edits::KeymapEditSink,
) -> Result<(), RegisterError> {
    for (id, request) in [
        (CommandId::KEYMAP_SHOW_EDITOR, KeymapEditorRequest::Show),
        (
            CommandId::KEYMAP_RECORD_BINDING,
            KeymapEditorRequest::Record,
        ),
    ] {
        let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        let sink = sink.clone();
        registry.register(meta, move |command: Command, _: HandlerContext| {
            let sink = sink.clone();
            async move {
                if command.id() != id {
                    return Err(OxiError::validation(format!("not a {id} command")));
                }
                sink.send(request)?;
                Ok(CommandOutput::none())
            }
        })?;
    }
    super::edits::register_edit_commands(registry, edits)
}

/// What opening the page needs.
#[derive(Clone)]
pub struct KeymapEditorViewsDeps {
    /// Where the page's buttons send their commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The window's cluster tabs.
    pub tabs: WeakEntity<ClusterTabs>,
    /// The window's own workspace (the one the catalog and the settings page open in): the page
    /// opens there when no cluster tab is open, so key bindings can be edited before any cluster
    /// is connected.
    pub home: WeakEntity<Workspace>,
}

/// The window's keymap editor page.
pub struct KeymapEditorViews;

impl KeymapEditorViews {
    /// Starts applying the requests of `requests` on the UI thread. Keep the returned task for the
    /// life of the window (dropping it stops it).
    pub fn start(
        deps: KeymapEditorViewsDeps,
        mut requests: UnboundedReceiver<KeymapEditorRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            while let Some(request) = requests.next().await {
                let opened = cx.update(|window, cx| {
                    let opened = Self::open(&deps, window, cx);
                    if opened && request == KeymapEditorRequest::Record {
                        Self::record(&deps, window, cx);
                    }
                    opened
                });
                if !matches!(opened, Ok(true)) {
                    // The window, or both its workspaces, are gone.
                    break;
                }
            }
        })
    }

    /// Shows the page in the active cluster's tab (the first open one when none is active) and
    /// brings the tab forward; with no cluster tab open, in the window's own workspace. `false`
    /// only when the window is gone.
    pub fn open(deps: &KeymapEditorViewsDeps, window: &mut Window, cx: &mut App) -> bool {
        let Some((cluster, workspace)) = Self::host(deps, cx) else {
            return false;
        };
        let key = SharedString::from(ITEM_KEY);
        match workspace.read(cx).find_item_by_key(&key, cx) {
            Some(item) => workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx)),
            None => {
                let page = KeymapEditorDeps {
                    dispatcher: deps.dispatcher.clone(),
                    workspace: Some(workspace.downgrade()),
                };
                let view = cx.new(|cx| KeymapEditorView::new(page, window, cx));
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
        if let (Some(cluster), Some(tabs)) = (cluster, deps.tabs.upgrade()) {
            tabs.update(cx, |tabs, cx| {
                tabs.activate(&cluster, window, cx);
            });
        }
        true
    }

    /// Opens the key recorder on the selected row of the page [`open`](Self::open) shows.
    /// `false` when there is no page or nothing is selected.
    pub fn record(deps: &KeymapEditorViewsDeps, window: &mut Window, cx: &mut App) -> bool {
        let Some((_, workspace)) = Self::host(deps, cx) else {
            return false;
        };
        let Some(page) = workspace
            .read(cx)
            .items_of_type::<KeymapEditorView>()
            .into_iter()
            .next()
        else {
            return false;
        };
        let opened = page.update(cx, |page, cx| page.record_selected(window, cx).is_some());
        if !opened {
            tracing::debug!("keymap::RecordBinding: select a row of the keymap editor first");
        }
        opened
    }

    /// The workspace the page goes in: the active cluster's tab (the first open one when none is
    /// active) with that cluster, else the window's own workspace.
    fn host(
        deps: &KeymapEditorViewsDeps,
        cx: &App,
    ) -> Option<(Option<ClusterId>, Entity<Workspace>)> {
        let in_tab = deps.tabs.upgrade().and_then(|tabs| {
            let tabs = tabs.read(cx);
            let cluster = tabs
                .active()
                .cloned()
                .or_else(|| tabs.clusters(cx).into_iter().next())?;
            let workspace = tabs.tab(&cluster)?.read(cx).workspace().clone();
            Some((Some(cluster), workspace))
        });
        in_tab.or_else(|| deps.home.upgrade().map(|home| (None, home)))
    }
}
