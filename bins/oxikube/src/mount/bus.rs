//! The command bus of the app: which crate registers which commands, and how views reach it.
//!
//! [`build_registry`] installs every handler the app has so far, each under the crate that owns
//! it, and [`BusDispatcher`] is the [`CommandDispatcher`] the views send their commands through:
//! it hands each command to the workspace's [`ClusterCommandRunner`], which dispatches on the bus
//! as `Initiator::Ui` off the UI thread and shows what came back (a toast, a confirmation
//! dialog, a denial). The palette, MCP and extensions will dispatch on the same bus. The cluster
//! tab commands and `namespace::Select` are immediate (E05-P600): the runner runs them in the
//! update that dispatched them, so their effect is in the next frame.
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
//! | `oxikube_app::exec` | `node::Shell` (guarded: blocked read-only, a confirmation naming the node and the image, a server dry run of the pod, audit; E09-S09) |
//! | `oxikube_palette` | `palette::Toggle` (the palette: opens or closes it, E11-S03), `palette::ToggleShowAll` (lists the commands that cannot run here too) |
//! | `oxikube_palette` | `palette::OpenJump` (the `:` jump bar, E11-S05), `jump::Back`, `jump::Forward`, `jump::Last` (its history: `[`, `]`, `-`) |
//! | `oxikube_workspace` (quit) | `app::Quit`: asks first while operations run, like `cmd-q` (E11-S05, for `:q`, the palette and agents) |
//! | `oxikube_terminal` | `terminal::OpenLink` (a terminal link's cmd/ctrl-click: a URL or local path, opened on the UI thread, E09-S05), `terminal::Copy` / `terminal::Paste` (dispatched to the focused terminal, E09-S06), `terminal::SelectAll` / `Clear` / `ScrollPageUp` / `ScrollPageDown` / `ScrollLineUp` / `ScrollLineDown` / `Search` / `SearchNext` / `SearchPrevious` / `SearchClose` (the same path, E09-S11), `terminal::New` / `Split` / `Close` (the window's terminal views: a shell in the shown cluster's bottom dock, a split, close the focused one; E09-S07) |
//!
//! `pod::Debug` (E09-S10, registered by `oxikube_terminal` over the app's `ExecService`) adds an
//! ephemeral debug container to a pod and opens a terminal in it.
//!
//! Only `resource::Delete`, `pod::Debug` and `node::Shell` (it creates a privileged pod) mutate a
//! cluster; they and the posture commands confirm and audit through the `MutationGuard` the bus
//! owns, and E12's per-kind actions join here.

use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc;
use gpui::{AnyWindowHandle, App};
use oxikube_app::catalog::ClusterCommandOutcome;
use oxikube_app::command_bus::{
    CommandOutput, CommandRegistry, HandlerContext, Immediate, RegisterError,
};
use oxikube_app::session::namespaces::NamespaceService;
use oxikube_app::{
    ClusterCommands, ClusterSessionManager, ExecService, KubeconfigSourcesService, PrefsWriter,
};
use oxikube_catalog_ui::catalog::CATALOG_VIEW;
use oxikube_catalog_ui::sources::SOURCES_VIEW;
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_logs_ui::LogCommandSink;
use oxikube_palette::command_palette::PaletteSink;
use oxikube_palette::jump::JumpSink;
use oxikube_resources_ui::ResourceCommandSink;
use oxikube_resources_ui::navigate::OpenKind;
use oxikube_terminal::input::TerminalInputSink;
use oxikube_terminal::open_link::LinkSink;
use oxikube_terminal::view::TerminalViewSink;
use oxikube_workspace::cluster_tab::CommandSink;
use oxikube_workspace::session::QuitSink;
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
    /// The app's exec service: `pod::Debug` adds its container through it (E09-S10), and
    /// `node::Shell`'s handler dry-runs the shell pod and leaves the permit the node's terminal
    /// opens with (E09-S09).
    pub exec: Arc<ExecService>,
    /// Where `palette::Toggle` and `palette::ToggleShowAll` send their request (applied on the UI
    /// thread by the window's palette host).
    pub palette: PaletteSink,
    /// Where `palette::OpenJump` and the jump history commands send their request (applied on the
    /// UI thread by the window's jump host).
    pub jump: JumpSink,
    /// Where `app::Quit` sends its request (applied on the UI thread).
    pub quit: QuitSink,
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
    registry.install("oxikube_palette", |r| {
        oxikube_palette::command_palette::register_commands(r, parts.palette)
    })?;
    registry.install("oxikube_palette::jump", |r| {
        oxikube_palette::jump::register_commands(r, parts.jump)
    })?;
    registry.install("oxikube_workspace::quit", |r| {
        oxikube_workspace::session::register_quit_command(r, parts.quit)
    })?;
    registry.install("oxikube_terminal", |r| {
        oxikube_terminal::open_link::register_commands(r, parts.links)?;
        oxikube_terminal::input::register_input_commands(r, parts.terminal_input)?;
        oxikube_terminal::view::register_view_commands(r, parts.terminal_views.clone())?;
        // `pod::Shell`, `pod::Attach`, `pod::Exec` (E09-S08): exec class, guarded and audited.
        oxikube_terminal::view::register_pod_commands(r, parts.terminal_views.clone())?;
        // `pod::Debug` (E09-S10): a guarded, confirmed, audited mutation that adds an ephemeral
        // container to the pod and opens a terminal attached to it.
        oxikube_terminal::view::register_debug_command(
            r,
            parts.terminal_views.clone(),
            parts.exec.clone(),
        )?;
        // `node::Shell` (E09-S09): a mutation (a privileged pod), confirmed with the node and the
        // image named, dry-run through the guard, audited at both ends of the pod's life.
        oxikube_app::exec::register_command(r, parts.exec, parts.terminal_views.node_shell_opener())
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
///
/// `namespace::Select` is immediate (E05-P600): the session's selection changes in the call, so
/// the UI's runner shows it in the frame after the input, and remembering it is the rest, run off
/// the UI thread. Its `changed` is whether the session's selection changed.
fn register_namespace_commands(
    registry: &mut CommandRegistry,
    service: NamespaceService,
) -> Result<(), RegisterError> {
    let select = service.clone();
    registry.register_immediate(
        meta(CommandId::NAMESPACE_SELECT)?,
        move |command: Command, _: HandlerContext| {
            let selected = select.execute_now(&command)?;
            let output = CommandOutput::data(json!({ "changed": selected.session_changed }));
            let remember = selected.remember();
            Ok(Immediate::then(output, async move {
                remember.await.map(|_| ())
            }))
        },
    )?;
    registry.register(
        meta(CommandId::NAMESPACE_TOGGLE_FAVOURITE)?,
        move |command: Command, _: HandlerContext| {
            let service = service.clone();
            async move {
                let outcome = service.execute(&command).await?;
                Ok(CommandOutput::data(json!({ "changed": outcome.changed })))
            }
        },
    )
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
