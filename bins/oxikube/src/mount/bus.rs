//! The command bus of the app: which crate registers which commands, and how views reach it.
//!
//! [`build_registry`] installs every handler the app has so far, each under the crate that owns
//! it, and [`BusDispatcher`] is the [`CommandDispatcher`] the views send their commands through:
//! it hands each command to the workspace's [`ClusterCommandRunner`], which dispatches on the bus
//! as `Initiator::Ui` off the UI thread and shows what came back (a toast, a confirmation
//! dialog, a denial). The palette, MCP and extensions will dispatch on the same bus.
//!
//! | Owner | Commands |
//! |---|---|
//! | `oxikube_app::catalog` | `cluster::Connect`, `Reconnect`, `CancelConnect`, `Disconnect`, `ToggleFavourite` |
//! | `oxikube_app::namespaces` | `namespace::Select`, `namespace::ToggleFavourite` |
//! | `oxikube_app::sources` | `kubeconfig::AddSource`, `RemoveSource`, `Reload` |
//! | `oxikube_app::posture` | `cluster::ToggleReadOnly`, `SetColour`, `ApplyPreset` (guarded posture) |
//! | `oxikube_workspace` | `cluster::Select`, `SwitchTab`, `NextTab`, `PreviousTab`, `CloseTab` |
//! | `oxikube` | `view::Open` for the catalog home and the kubeconfig sources screen |
//! | `oxikube_app::actions` | `resource::Delete` (guarded: read-only check, confirm tier by target, server dry run, audit; E07-S08) |
//! | `oxikube_resources_ui` | `resource::OpenList` (read-only navigation to a kind's list, E07-S11); `resource::Open`, `resource::CopyName`, `resource::SelectAll` (the resource tables, E07-S03), `resource::RetryFeed` (restart a table's feed, E07-S10) |
//! | `oxikube_logs_ui` | `pod::ViewLogs` (open a pod's log view), `workload::ViewLogs` (a workload's or Service's pods merged, E08-S04) and the log view's `logs::SetRange`, `SelectContainer`, `TogglePrevious`, `ToggleWrap`, `ToggleTimestamps`, `ToggleAutoscroll`, `ToggleFullscreen`, `ToggleSource` (E08-S02, S04; reads only), and its search's `logs::Find`, `NextMatch`, `PreviousMatch`, `ToggleCase`, `ToggleInverse`, `ToggleFilterMode`, `CloseSearch` (E08-S03; reads only) |
//! | `oxikube_terminal` | `terminal::OpenLink` (a terminal link's cmd/ctrl-click: a URL or local path, opened on the UI thread, E09-S05), `terminal::Copy` / `terminal::Paste` (dispatched to the focused terminal, E09-S06), `terminal::New` / `Split` / `Close` (the window's terminal views: a shell in the shown cluster's bottom dock, a split, close the focused one; E09-S07) |
//!
//! Only `resource::Delete` mutates a cluster; it and the posture commands confirm and audit through
//! the `MutationGuard` the bus owns, and E12's per-kind actions join here.

use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc;
use gpui::{AnyWindowHandle, App};
use oxikube_app::catalog::ClusterCommandOutcome;
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::{ClusterCommands, ClusterSessionManager, KubeconfigSourcesService, PrefsWriter};
use oxikube_catalog_ui::catalog::CATALOG_VIEW;
use oxikube_catalog_ui::sources::SOURCES_VIEW;
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_logs_ui::LogCommandSink;
use oxikube_resources_ui::ResourceCommandSink;
use oxikube_resources_ui::navigate::OpenKind;
use oxikube_terminal::input::TerminalInputSink;
use oxikube_terminal::open_link::LinkSink;
use oxikube_terminal::view::TerminalViewSink;
use oxikube_workspace::cluster_tab::CommandSink;
use oxikube_workspace::{ClusterCommandRunner, CommandDispatcher};
use serde_json::json;

/// The views `view::Open` opens in the main window.
pub const VIEWS: [&str; 2] = [CATALOG_VIEW, SOURCES_VIEW];

/// What the registry's handlers run on.
pub struct BusParts {
    /// The cluster commands' handler.
    pub cluster_commands: ClusterCommands,
    /// The namespace commands' service.
    pub namespaces: NamespaceService,
    /// The kubeconfig sources service.
    pub sources: KubeconfigSourcesService,
    /// The sessions the posture commands change.
    pub sessions: ClusterSessionManager,
    /// Where the posture commands persist (`settings.json`).
    pub prefs: Arc<dyn PrefsWriter>,
    /// The cluster tab controller's queue.
    pub tabs: CommandSink,
    /// Where `view::Open` sends the view to open (applied on the UI thread).
    pub views: mpsc::UnboundedSender<String>,
    /// Where `resource::OpenList` sends the list to open (applied on the UI thread).
    pub kinds: mpsc::UnboundedSender<OpenKind>,
    /// The resource views' queue (the table commands, applied on the UI thread).
    pub resources: ResourceCommandSink,
    /// The log views' queue (`pod::ViewLogs` and `logs::*`, applied on the UI thread).
    pub logs: LogCommandSink,
    /// Where `terminal::OpenLink` sends the links to open (opened on the UI thread).
    pub links: LinkSink,
    /// Where `terminal::Copy` and `terminal::Paste` send what the focused terminal should do.
    pub terminal_input: TerminalInputSink,
    /// The terminal views' queue (`terminal::New`, `Split`, `Close`, applied on the UI thread).
    pub terminal_views: TerminalViewSink,
}

/// Every handler of the app, each installed under its owner (see the [module docs](self)).
///
/// # Errors
///
/// A [`RegisterError`] when two crates register one id: a wiring bug, caught by the tests.
pub fn build_registry(parts: BusParts) -> Result<CommandRegistry, RegisterError> {
    let mut registry = CommandRegistry::new();
    registry.install("oxikube_app::catalog", |r| {
        register_cluster_commands(r, parts.cluster_commands)
    })?;
    registry.install("oxikube_app::namespaces", |r| {
        register_namespace_commands(r, parts.namespaces)
    })?;
    registry.install("oxikube_app::sources", |r| {
        oxikube_app::sources::register_commands(r, &parts.sources)
    })?;
    registry.install("oxikube_app::posture", |r| {
        oxikube_app::guard::register_commands(r, parts.sessions, parts.prefs)
    })?;
    registry.install("oxikube_workspace", |r| {
        oxikube_workspace::cluster_tab::register_commands(r, parts.tabs)
    })?;
    registry.install("oxikube", |r| register_view_commands(r, parts.views))?;
    registry.install(
        "oxikube_app::actions",
        oxikube_app::actions::register_commands,
    )?;
    registry.install("oxikube_resources_ui", |r| {
        oxikube_resources_ui::navigate::register_commands(r, parts.kinds)?;
        oxikube_resources_ui::register_commands(r, parts.resources)
    })?;
    registry.install("oxikube_logs_ui", |r| {
        oxikube_logs_ui::register_commands(r, parts.logs)
    })?;
    registry.install("oxikube_terminal", |r| {
        oxikube_terminal::open_link::register_commands(r, parts.links)?;
        oxikube_terminal::input::register_input_commands(r, parts.terminal_input)?;
        oxikube_terminal::view::register_view_commands(r, parts.terminal_views.clone())?;
        // `pod::Shell`, `pod::Attach`, `pod::Exec` (E09-S08): exec class, guarded and audited.
        oxikube_terminal::view::register_pod_commands(r, parts.terminal_views)
    })?;
    Ok(registry)
}

fn meta(id: CommandId) -> Result<command::CommandMeta, RegisterError> {
    command::lookup(id)
        .copied()
        .ok_or(RegisterError::Undeclared(id))
}

/// `cluster::Connect`, `Reconnect`, `CancelConnect`, `Disconnect`, `ToggleFavourite` over
/// [`ClusterCommands`]. A connect's output is the state it ended in; a failed connection is an
/// `Ok` with that state, which the cluster's tab shows.
fn register_cluster_commands(
    registry: &mut CommandRegistry,
    commands: ClusterCommands,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::CLUSTER_CONNECT,
        CommandId::CLUSTER_RECONNECT,
        CommandId::CLUSTER_CANCEL_CONNECT,
        CommandId::CLUSTER_DISCONNECT,
        CommandId::CLUSTER_TOGGLE_FAVOURITE,
    ] {
        let commands = commands.clone();
        registry.register(meta(id)?, move |command: Command, _: HandlerContext| {
            let commands = commands.clone();
            async move {
                let data = match commands.handle(&command).await? {
                    ClusterCommandOutcome::Connected(state) => {
                        json!({ "phase": format!("{:?}", state.phase()) })
                    }
                    ClusterCommandOutcome::Cancelled(cancelled) => {
                        json!({ "cancelled": cancelled })
                    }
                    ClusterCommandOutcome::Disconnected => json!({ "disconnected": true }),
                    ClusterCommandOutcome::Favourite(favourite) => {
                        json!({ "favourite": favourite })
                    }
                };
                Ok(CommandOutput::data(data))
            }
        })?;
    }
    Ok(())
}

/// `namespace::Select` and `namespace::ToggleFavourite` over the [`NamespaceService`].
fn register_namespace_commands(
    registry: &mut CommandRegistry,
    service: NamespaceService,
) -> Result<(), RegisterError> {
    for id in [
        CommandId::NAMESPACE_SELECT,
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
    ] {
        let service = service.clone();
        registry.register(meta(id)?, move |command: Command, _: HandlerContext| {
            let service = service.clone();
            async move {
                let outcome = service.execute(&command).await?;
                Ok(CommandOutput::data(json!({ "changed": outcome.changed })))
            }
        })?;
    }
    Ok(())
}

/// `view::Open` for the [`VIEWS`] of the main window: the view id goes to `views`, whose receiver
/// opens it on the UI thread.
fn register_view_commands(
    registry: &mut CommandRegistry,
    views: mpsc::UnboundedSender<String>,
) -> Result<(), RegisterError> {
    registry.register(
        meta(CommandId::VIEW_OPEN)?,
        move |command: Command, _: HandlerContext| {
            let views = views.clone();
            async move {
                let Command::ViewOpen { view } = command else {
                    return Err(OxiError::validation("not a view::Open command"));
                };
                if !VIEWS.contains(&view.as_str()) {
                    return Err(OxiError::not_found(format!(
                        "no view `{view}` (known: {})",
                        VIEWS.join(", ")
                    )));
                }
                views
                    .unbounded_send(view)
                    .map_err(|_| OxiError::internal("the main window is gone"))?;
                Ok(CommandOutput::none())
            }
        },
    )
}

/// The [`CommandDispatcher`] of the main window's views: every command goes to the bus through
/// the window's [`ClusterCommandRunner`] (set once the bus exists, right after the cluster tabs
/// it routes to). Cheap to clone.
#[derive(Clone)]
pub struct BusDispatcher {
    runner: Rc<OnceCell<ClusterCommandRunner>>,
    window: AnyWindowHandle,
}

impl BusDispatcher {
    /// A dispatcher for `window`, with no runner yet.
    pub fn new(window: AnyWindowHandle) -> Self {
        Self {
            runner: Rc::default(),
            window,
        }
    }

    /// Sets the runner. The first one stays.
    pub fn set_runner(&self, runner: ClusterCommandRunner) {
        if self.runner.set(runner).is_err() {
            tracing::warn!("the main window's command runner was set twice");
        }
    }
}

impl CommandDispatcher for BusDispatcher {
    fn dispatch(&self, command: Command, cx: &mut App) {
        let Some(runner) = self.runner.get().cloned() else {
            tracing::warn!(command = %command.id(), "no command bus yet: command dropped");
            return;
        };
        let window = self.window;
        // Deferred: views dispatch from inside their own update of this window, and the runner
        // needs the window.
        cx.defer(move |cx| {
            let id = command.id();
            if window
                .update(cx, |_, window, cx| runner.run(command, window, cx))
                .is_err()
            {
                tracing::debug!(command = %id, "the main window is gone: command dropped");
            }
        });
    }
}
