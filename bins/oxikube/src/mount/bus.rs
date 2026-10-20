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
//! | `oxikube` | `view::Open` for the catalog home, the kubeconfig sources screen and the chart gallery, `charts::OpenGallery` (the gallery's palette command, E13-S07); `keymap::OpenUser` (creates `keymap.json` from its template if missing and opens it, E11-S08) |
//! | `oxikube_app::actions` | `resource::Delete` (guarded: read-only check, confirm tier by target, server dry run, audit; E07-S08) |
//! | `oxikube_app::selection` | `selection::Delete`, `selection::Restart`, `selection::Label` (E12-S19: the bulk actions on a table's selection; guarded as one command: blocked read-only before any request, one confirmation at the highest tier among the objects, a server dry run per object, a result and one audit record per object under a shared batch id) |
//! | `oxikube_app::nodes` | `node::Cordon`, `node::Uncordon` (guarded: blocked read-only, risk low so one click confirms, a server dry run of the `spec.unschedulable` patch before the real one, audit with the node and the initiator; E12-S09); `node::Drain` (guarded: blocked read-only, risk high so the node's name is typed, `kubectl drain` through the kube adapter's `KubeDrain` with the guard's writer, one audit record for the drain and one per pod sharing a batch id; E12-S10) |
//! | `oxikube_app::apply` | `resource::Apply` (guarded: blocked read-only, risk low with the editor's diff preview as the confirmation, a server dry run before every apply, server-side apply as the `editor.field_manager`, the replace fallback at the reviewed `resourceVersion`, audit; E10-S08). The manifest editor runs it through `ApplyService` |
//! | `oxikube_app::workloads` | `cronjob::Trigger` (a Job from the CronJob's template), `cronjob::Suspend` / `cronjob::Resume`, `job::Suspend` / `job::Resume` (guarded: blocked read-only, risk low so a simple confirm, a server dry run first, audit with the created Job's name; E12-S08), `workload::Scale` (E12-S04: the `scale` subresource, risk low and medium for zero, audit with `before=` and `after=`), `workload::Rollback` (E12-S06: `kubectl rollout undo` from a Deployment's ReplicaSets or a StatefulSet's / DaemonSet's ControllerRevisions, risk medium, a server dry run first, audit with `from=` and `to=`), `workload::Restart` and `workload::SetImage` (E12-S05: the pod template's `restartedAt` merge patch and a strategic image patch, risk low, audit with the time or the old and new images and the server's `rv=`), `pod::Evict`, `pod::Delete` (risk medium, a simple confirm naming the pod), `pod::ForceDelete` (high: the pod's name typed), `pod::Sanitize` (the Completed and Failed pods of a view; one audit record per pod under a batch id; E12-S03) |
//! | `oxikube_resources_ui` | `resource::OpenList` (read-only navigation to a kind's list, E07-S11); `resource::Open`, `resource::CopyName`, `resource::SelectAll` (the resource tables, E07-S03), `resource::RetryFeed` (restart a table's feed, E07-S10), `secret::Reveal` (a Secret's values decoded in memory in its detail, `x`, E12-S11); `pod::JumpToOwner`, `pod::ShowNode` (read the pod, open its owner or node in the drawer; E12-S03) |
//! | `oxikube_logs_ui` | `pod::ViewLogs` (open a pod's log view), `workload::ViewLogs` (a workload's or Service's pods merged, E08-S04) and the log view's `logs::SetRange`, `SelectContainer`, `TogglePrevious`, `ToggleWrap`, `ToggleTimestamps`, `ToggleAutoscroll`, `ToggleFullscreen`, `ToggleSource` (E08-S02, S04; reads only), and its search's `logs::Find`, `NextMatch`, `PreviousMatch`, `ToggleCase`, `ToggleInverse`, `ToggleFilterMode`, `CloseSearch` (E08-S03; reads only) |
//! | `oxikube_app::tools::events` | `events::SendToAgent` (the selected events, an object's events or the newest warnings, queued as agent context from the `EventService`'s buffer; reads only; E14-S08) |
//! | `oxikube_app::exec` | `node::Shell` (guarded: blocked read-only, a confirmation naming the node and the image, a server dry run of the pod, audit; E09-S09) |
//! | `oxikube_palette` | `palette::Toggle` (the palette: opens or closes it, E11-S03), `palette::ToggleShowAll` (lists the commands that cannot run here too) |
//! | `oxikube_palette` | `palette::OpenJump` (the `:` jump bar, E11-S05), `jump::Back`, `jump::Forward`, `jump::Last` (its history: `[`, `]`, `-`) |
//! | `oxikube_workspace` (quit) | `app::Quit`: asks first while operations run, like `cmd-q` (E11-S05, for `:q`, the palette and agents) |
//! | `oxikube_palette` | `help::Show` (the `?` overlay of the keys that apply where the focus is; read-only, E11-S10) |
//! | `oxikube_app::metrics_provider` | `cluster::RedetectMetrics` (run provider detection again, or re-check the configured endpoint), `cluster::ToggleMetrics` (`prometheus.enabled` in `settings.json`); neither changes a cluster (E13-S05) |
//! | `oxikube_app::os_notifications` | `app::ToggleOsNotifications` (the `events.os_notifications` opt-in in `settings.json`), `app::SendTestNotification` (one desktop notification, only once opted in); neither changes a cluster (E14-S06) |
//! | `oxikube_app::notifications` | `notifications::MarkRead`, `notifications::MarkAllRead`, `notifications::MuteCluster`, `notifications::UnmuteCluster` (the notification centre's read state and the per-cluster mute, kept in the state db; no cluster is changed; E14-S05) |
//! | `oxikube_app::portforward` | `portforward::Start`, `Stop`, `Restart`, `Remove` (the app's one `PortForwardManager`: forwards to pods and Services that follow pod churn; refused on a read-only cluster by default; no cluster is changed; E15-S03) and `service::OpenInBrowser` (the manager as the Service port opener, the URL to the system browser); `portforward::FavouriteAdd`, `FavouriteRemove`, `FavouriteToggleAutoStart` (saved forwards in the state db, restored when the cluster's session becomes Ready; E15-S06) |
//! | `oxikube_app::portforward` | `portforward::Start`, `Stop`, `Restart`, `Remove` (the app's one `PortForwardManager`: forwards to pods and Services that follow pod churn; refused on a read-only cluster by default; no cluster is changed; E15-S03) and `service::OpenInBrowser` (the manager as the Service port opener, the URL to the system browser) |
//! | `oxikube_portforward_ui` | `portforward::CopyUrl`, `OpenInBrowser` (the forward's local URL to the clipboard, to the system browser) and `TogglePanel` (the Port Forwards panel in the bottom dock: `:pf`, the table's `f`, the palette; no cluster is changed; E15-S05) |
//! | `oxikube_workspace::notifications` | `notifications::TogglePanel` (the notification centre panel in the right dock; E14-S05) |
//! | `oxikube_settings_ui` | `cluster::ShowMetricsSettings` (the cluster's metrics page, a tab of its tab; E13-S05) |
//! | `oxikube_overview_ui` | `cluster::ShowPulses` (the cluster's pulses page, `:pulses`; E13-S10) |
//! | `oxikube_overview_ui` | `cluster::ShowOverview` (the cluster's overview, a tab of its tab; reads only; E13-S09) |
//! | `oxikube_events_ui` | `events::Open` (the cluster's events view, `:ev`, the sidebar's Events heading), `events::ToggleFaults`, `TogglePause`, `CopyRow`, `CopyRowJson`, `ExportJson` (the open view's; read the app's `EventService`, never the cluster; E14-S03) |
//! | `oxikube_editor` | `editor::NewManifest` (an empty manifest editor in the shown cluster's tab, its YAML checked against that cluster's schemas), `resource::Create` (the "Create resource" picker, or one template, opened in a new editor with the cluster's selected namespace filled in; E10-S09: it writes nothing, the objects are made by `resource::Apply`, one per document), `editor::ToggleReadOnly`, `editor::ToggleSoftWrap` (the focused editor's view toggles; E10-S04), `editor::TriggerCompletion` / `editor::ShowHover` (the focused editor's completion menu and field docs at the cursor, from the cluster's schema; E10-S05), `resource::Edit`, `resource::ViewYamlInEditor` (a `kind/name` editor tab on a freshly read object, E10-S06), `editor::ToggleManagedFields`, `editor::ToggleSecretMask` (E10-S06), `editor::ConvertToJson`, `editor::ConvertToYaml` (the buffer as the other language, one undo step), `editor::UseJson`, `editor::UseYaml` (the language the buffer is read as; E10-S10), `secret::EditDecoded` (a Secret in an editor tab with its values as text, base64-encoded again by `resource::Apply`; E12-S11), `secret::Create` (the "Create secret" dialog: Opaque, Docker registry or TLS, validated before an editor tab opens with the manifest; E12-S11), `editor::ShowDiff` / `ToggleDiffLayout` / `ReloadFromServer` / `MergeFromServer` / `KeepMine` / `TakeMine` / `TakeServer` / `AcceptMerge` (diff vs live and the changed-on-server banner of the focused editor: read the object, change the buffer only; E10-S07) |
//! | `oxikube_terminal` | `terminal::OpenLink` (a terminal link's cmd/ctrl-click: a URL or local path, opened on the UI thread, E09-S05), `terminal::Copy` / `terminal::Paste` (dispatched to the focused terminal, E09-S06), `terminal::SelectAll` / `Clear` / `ScrollPageUp` / `ScrollPageDown` / `ScrollLineUp` / `ScrollLineDown` / `Search` / `SearchNext` / `SearchPrevious` / `SearchClose` (the same path, E09-S11), `terminal::New` / `Split` / `Close` (the window's terminal views: a shell in the shown cluster's bottom dock, a split, close the focused one; E09-S07) |
//!
//! `pod::Debug` (E09-S10, registered by `oxikube_terminal` over the app's `ExecService`) adds an
//! ephemeral debug container to a pod and opens a terminal in it.
//!
//! `resource::Delete`, `resource::Apply`, `pod::Debug`, `node::Shell` (it creates a privileged
//! pod), `workload::Scale`, `workload::Restart`, `workload::SetImage`, `workload::Rollback`, the `cronjob::*` / `job::*` commands and the `pod::Evict`, `pod::Delete`,
//! `pod::ForceDelete`, `pod::Sanitize` and `selection::*` commands mutate a cluster; they and the posture commands
//! confirm and audit through the `MutationGuard` the bus owns, and E12's other per-kind actions
//! join here.

use std::cell::OnceCell;
use std::rc::Rc;
use std::sync::Arc;

use super::gallery::CHART_GALLERY_VIEW;
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
use oxikube_editor::view::EditorViewSink;
use oxikube_logs_ui::LogCommandSink;
use oxikube_palette::command_palette::PaletteSink;
use oxikube_palette::help::HelpSink;
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
pub const VIEWS: [&str; 3] = [CATALOG_VIEW, SOURCES_VIEW, CHART_GALLERY_VIEW];

/// What the registry's handlers run on.
pub struct BusParts {
    /// What `events::SendToAgent` reads (the app's `EventService`; E14-S08).
    pub events_agent: oxikube_app::tools::events::EventsAgentDeps,
    /// The queue `events::SendToAgent` fills until the agent panel takes it.
    pub agent_pending: oxikube_app::context::PendingContext,
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
    /// The user's `keymap.json`; `None` without a config directory.
    pub keymap_file: Option<std::path::PathBuf>,
    /// Where `keymap::OpenUser` sends the file to open (applied on the UI thread).
    pub keymap_open: super::keymap::OpenSink,
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
    /// Where `help::Show` sends its request (the window shows the overlay on the UI thread).
    pub help: HelpSink,
    /// The node service `node::Drain` runs on (E12-S10): the kube adapter's drain algorithm, the
    /// followers of the drain dialog, the per-pod audit records.
    pub nodes: Arc<oxikube_app::nodes::NodeService>,
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
    /// The manifest editors' queue (`editor::*`, applied on the UI thread).
    pub editors: EditorViewSink,
    /// The metrics provider service (`cluster::RedetectMetrics`, `cluster::ToggleMetrics`; E13-S05).
    pub metrics: oxikube_app::MetricsProviderService,
    /// Where `cluster::ToggleMetrics` persists (`settings.json`).
    pub metrics_settings: Arc<dyn oxikube_app::metrics_provider::MetricsSettingsWriter>,
    /// Where `cluster::ShowMetricsSettings` sends the cluster whose page to open (applied on the UI
    /// thread).
    pub metrics_pages: oxikube_settings_ui::metrics::MetricsRequestSink,
    /// Where `cluster::ShowPulses` sends the cluster whose pulses to open (applied on the UI
    /// thread; E13-S10).
    pub pulses_pages: oxikube_overview_ui::pulses::PulsesRequestSink,
    /// Where `cluster::ShowOverview` sends the cluster whose overview to open (applied on the UI
    /// thread; E13-S09).
    pub overview_pages: oxikube_overview_ui::OverviewRequestSink,
    /// The desktop notification service (`app::ToggleOsNotifications`, `app::SendTestNotification`;
    /// E14-S06).
    pub os_notifications: oxikube_app::OsNotificationService,
    /// Where `app::ToggleOsNotifications` persists (`settings.json`).
    pub os_notification_settings: Arc<dyn oxikube_app::os_notifications::OsNotificationSettings>,
    /// Where the `events::*` commands send their requests (applied on the UI thread; E14-S03).
    pub events_views: oxikube_events_ui::EventsCommandSink,
    /// The notification centre (`notifications::MarkRead`, `MarkAllRead`, `MuteCluster`,
    /// `UnmuteCluster`; E14-S05).
    pub notifications: oxikube_app::NotificationService,
    /// Where `notifications::TogglePanel` sends its requests (applied on the UI thread; E14-S05).
    pub notification_panel: oxikube_workspace::notifications::NotificationPanelSink,
    /// The port-forward manager (`portforward::Start`, `Stop`, `Restart`, `Remove`, and the
    /// opener behind `service::OpenInBrowser`; E15-S03).
    pub port_forwards: oxikube_app::PortForwardManager,
    /// The saved port-forwards (`portforward::FavouriteAdd`, `FavouriteRemove`,
    /// `FavouriteToggleAutoStart`; E15-S06).
    pub port_forward_favourites: oxikube_app::portforward::PortForwardFavourites,
    /// Where `portforward::CopyUrl`, `OpenInBrowser` and `TogglePanel` send their requests
    /// (applied on the UI thread; E15-S05).
    pub port_forward_ui: oxikube_portforward_ui::PortForwardUiSink,
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
    registry.install("oxikube_app::session::namespaces", |r| {
        register_namespace_commands(r, parts.namespaces)
    })?;
    // `namespace::Create` and `namespace::Delete` (E12-S16): guarded mutations of the Namespace
    // kind (a server dry run first; delete takes the typed name and is audited with the counts).
    registry.install(
        "oxikube_app::namespaces",
        oxikube_app::namespaces::register_commands,
    )?;
    registry.install("oxikube_app::sources", |r| {
        oxikube_app::sources::register_commands(r, &parts.sources)
    })?;
    registry.install("oxikube_app::apply", |r| {
        oxikube_app::apply::register_commands(r, parts.sessions.clone())
    })?;
    // `ingressclass::SetDefault` (E12-S14): a low-risk guarded mutation, the Network section's only
    // one; it reads the cluster's classes to clear the flag on the previous default.
    registry.install("oxikube_app::ingress_class", |r| {
        oxikube_app::ingress_class::register_commands(r, parts.sessions.clone())
    })?;
    registry.install("oxikube_app::workloads", |r| {
        oxikube_app::workloads::register_commands(r, parts.sessions.clone())
    })?;
    // `selection::Delete`, `selection::Restart` and `selection::Label` (E12-S19): the bulk actions on
    // a table's selection, one guarded command each.
    registry.install("oxikube_app::selection", |r| {
        oxikube_app::selection::register_commands(r, parts.sessions.clone())
    })?;
    // `pvc::Resize` and `storageclass::SetDefault` (E12-S15): guarded mutations over the sessions.
    registry.install("oxikube_app::storage", |r| {
        oxikube_app::storage::register_commands(r, parts.sessions.clone())
    })?;
    registry.install("oxikube_app::posture", |r| {
        oxikube_app::guard::register_commands(r, parts.sessions, parts.prefs)
    })?;
    registry.install("oxikube_workspace", |r| {
        oxikube_workspace::cluster_tab::register_commands(r, parts.tabs)
    })?;
    registry.install("oxikube", |r| {
        register_view_commands(r, parts.views)?;
        super::keymap::register_commands(r, parts.keymap_file, parts.keymap_open)
    })?;
    registry.install(
        "oxikube_app::actions",
        oxikube_app::actions::register_commands,
    )?;
    // `node::Cordon` and `node::Uncordon` (E12-S09): guarded, confirmed once, audited; `node::Drain`
    // (E12-S10): guarded, the node's name typed, the drain and each pod audited.
    registry.install("oxikube_app::nodes", |r| {
        oxikube_app::nodes::register_commands(r)?;
        oxikube_app::nodes::register_drain_command(r, parts.nodes)
    })?;
    registry.install("oxikube_portforward_ui", |r| {
        oxikube_portforward_ui::register_commands(
            r,
            Arc::new(parts.port_forwards.clone()),
            parts.port_forward_ui,
        )
    })?;
    registry.install("oxikube_app::portforward", |r| {
        oxikube_app::portforward::register_commands(r, parts.port_forwards.clone())?;
        oxikube_app::portforward::favourites::register_commands(r, parts.port_forward_favourites)?;
        oxikube_resources_ui::kinds::service::register_open_in_browser(
            r,
            Arc::new(parts.port_forwards),
            parts.resources.clone(),
        )
    })?;
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
    registry.install("oxikube_palette::help", |r| {
        oxikube_palette::help::register_commands(r, parts.help)
    })?;
    registry.install("oxikube_app::metrics_provider", |r| {
        oxikube_app::metrics_provider::register_commands(r, parts.metrics, parts.metrics_settings)
    })?;
    registry.install("oxikube_app::os_notifications", |r| {
        oxikube_app::os_notifications::register_commands(
            r,
            parts.os_notifications,
            parts.os_notification_settings,
        )
    })?;
    registry.install("oxikube_app::notifications", |r| {
        oxikube_app::notifications::register_commands(r, parts.notifications)
    })?;
    registry.install("oxikube_workspace::notifications", |r| {
        oxikube_workspace::notifications::register_commands(r, parts.notification_panel)
    })?;
    registry.install("oxikube_settings_ui", |r| {
        oxikube_settings_ui::metrics::register_commands(r, parts.metrics_pages)
    })?;
    registry.install("oxikube_overview_ui", |r| {
        oxikube_overview_ui::pulses::register_commands(r, parts.pulses_pages)?;
        oxikube_overview_ui::register_commands(r, parts.overview_pages)
    })?;
    registry.install("oxikube_app::tools::events", |r| {
        oxikube_app::tools::events::register_commands(r, &parts.events_agent, parts.agent_pending)
    })?;
    registry.install("oxikube_events_ui", |r| {
        oxikube_events_ui::register_commands(r, parts.events_views)
    })?;
    registry.install("oxikube_editor", |r| {
        oxikube_editor::view::register_commands(r, parts.editors)
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

/// `namespace::Select`, `namespace::ToggleFavourite` and `namespace::UseFavourite` over the
/// [`NamespaceService`].
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
    // The number keys (E12-S16): `0` is All, `1`-`9` the nth favourite. Immediate like Select
    // when the favourites are loaded (the selector loads them when the cluster tab opens); else
    // the rest of the command loads them off the UI thread and selects.
    let digits = service.clone();
    registry.register_immediate(
        meta(CommandId::NAMESPACE_USE_FAVOURITE)?,
        move |command: Command, _: HandlerContext| {
            let Command::NamespaceUseFavourite { cluster, slot } = command else {
                return Err(OxiError::validation(
                    "not a namespace::UseFavourite command",
                ));
            };
            match digits.use_favourite_now(&cluster, slot)? {
                Some(selected) => {
                    let output =
                        CommandOutput::data(json!({ "changed": selected.session_changed }));
                    let remember = selected.remember();
                    Ok(Immediate::then(output, async move {
                        remember.await.map(|_| ())
                    }))
                }
                None => {
                    let digits = digits.clone();
                    Ok(Immediate::then(
                        CommandOutput::data(json!({ "deferred": true })),
                        async move { digits.use_favourite(&cluster, slot).await.map(|_| ()) },
                    ))
                }
            }
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

/// `view::Open` for the [`VIEWS`] of the main window, and `charts::OpenGallery`: the view id goes
/// to `views`, whose receiver opens it on the UI thread.
fn register_view_commands(
    registry: &mut CommandRegistry,
    views: mpsc::UnboundedSender<String>,
) -> Result<(), RegisterError> {
    let gallery = views.clone();
    registry.register(
        meta(CommandId::CHARTS_OPEN_GALLERY)?,
        move |_: Command, _: HandlerContext| {
            let gallery = gallery.clone();
            async move {
                gallery
                    .unbounded_send(CHART_GALLERY_VIEW.to_owned())
                    .map_err(|_| OxiError::internal("the main window is gone"))?;
                Ok(CommandOutput::none())
            }
        },
    )?;
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
