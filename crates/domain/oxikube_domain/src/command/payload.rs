//! The [`Command`] payload enum: navigation plus resource verbs.

use serde::{Deserialize, Serialize};

use super::id::CommandId;
use super::kubeconfig::{KubeconfigSourceRef, NewKubeconfigSource};
use super::meta::CommandMeta;
use super::registry;
use crate::colour::ClusterColour;
use crate::ids::{ClusterId, Gvk, ResourceRef};
use crate::log::{LevelChip, LogRange, LogSaveScope};
use crate::preset::ClusterPreset;

/// How the API server deletes dependents of an object.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Propagation {
    /// Delete the object now; the garbage collector removes dependents later.
    #[default]
    Background,
    /// Delete dependents first, then the object.
    Foreground,
    /// Leave dependents in place.
    Orphan,
}

/// `true`: the serde default of flags that are on unless the caller says otherwise.
fn default_follow() -> bool {
    true
}

/// A user or agent action as plain data.
///
/// Serialised as JSON with a `type` tag equal to the command id
/// (`{"type":"workload::Scale","target":{..},"replicas":3}`), so an MCP tool
/// call or a keymap action with arguments converts to a `Command` with one
/// `serde_json::from_value`. Variants hold no closures and no UI types.
///
/// Each variant has exactly one [`CommandId`] ([`Command::id`]) and one
/// [`CommandMeta`] ([`Command::meta`]); see the module docs for the keymap /
/// command / tool mapping. Resource verbs act on a [`ResourceRef`], which
/// already carries the cluster, so they are valid regardless of which cluster
/// is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    /// Connect a cluster: open its session and run discovery. Reads from the cluster, never
    /// changes it. Connecting an already connected cluster is a no-op.
    #[serde(rename = "cluster::Connect")]
    ClusterConnect {
        /// The catalog entry to connect.
        cluster: ClusterId,
    },
    /// Cancel a connection attempt that is still in flight (the connect view's Cancel). Does
    /// nothing once the attempt has ended.
    #[serde(rename = "cluster::CancelConnect")]
    ClusterCancelConnect {
        /// The cluster whose attempt to cancel.
        cluster: ClusterId,
    },
    /// Close a cluster's workspace tab, which disconnects the cluster. Asks first while
    /// operations of that cluster (exec sessions, port-forwards) are running.
    #[serde(rename = "cluster::CloseTab")]
    ClusterCloseTab {
        /// The cluster whose tab closes.
        cluster: ClusterId,
    },
    /// Disconnect a cluster: cancel an attempt in flight or close its connection.
    #[serde(rename = "cluster::Disconnect")]
    ClusterDisconnect {
        /// The catalog entry to disconnect.
        cluster: ClusterId,
    },
    /// Reconnect a cluster: drop its connection, if any, and connect again. The retry of the
    /// connect view. Reads from the cluster, never changes it.
    #[serde(rename = "cluster::Reconnect")]
    ClusterReconnect {
        /// The cluster to reconnect.
        cluster: ClusterId,
    },
    /// Show the next cluster tab (wraps around).
    #[serde(rename = "cluster::NextTab")]
    ClusterNextTab,
    /// Show the previous cluster tab (wraps around).
    #[serde(rename = "cluster::PreviousTab")]
    ClusterPreviousTab,
    /// Make a cluster the active one: its tab is shown.
    #[serde(rename = "cluster::Select")]
    ClusterSelect {
        /// The cluster to activate.
        cluster: ClusterId,
    },
    /// Show the nth cluster tab (`cmd-1` to `cmd-9`).
    #[serde(rename = "cluster::SwitchTab")]
    ClusterSwitchTab {
        /// The tab, counted from 1 in tab order.
        index: u8,
    },
    /// Mark a cluster as a favourite (`Some(true)`), clear it (`Some(false)`) or flip it
    /// (`None`). A favourite sorts to the top of the catalog; the flag is local state.
    #[serde(rename = "cluster::ToggleFavourite")]
    ClusterToggleFavourite {
        /// The catalog entry to change.
        cluster: ClusterId,
        /// Desired state; `None` flips the current one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        favourite: Option<bool>,
    },
    /// Set (`Some`) or toggle (`None`) a cluster's read-only mode.
    #[serde(rename = "cluster::ToggleReadOnly")]
    ClusterToggleReadOnly {
        /// The cluster to change.
        cluster: ClusterId,
        /// Desired state; `None` flips the current one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_only: Option<bool>,
    },
    /// Set (`Some`) or clear (`None`) a cluster's accent colour.
    ///
    /// Not a cluster mutation: it only edits the cluster's own settings, so it runs on a
    /// read-only cluster. It is audited like a posture change.
    #[serde(rename = "cluster::SetColour")]
    ClusterSetColour {
        /// The cluster to change.
        cluster: ClusterId,
        /// The colour; `None` clears it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<ClusterColour>,
    },
    /// Give a cluster a named posture: its colour and, for production, read-only mode on.
    /// A preset never lowers protection (see [`ClusterPreset`]).
    #[serde(rename = "cluster::ApplyPreset")]
    ClusterApplyPreset {
        /// The cluster to change.
        cluster: ClusterId,
        /// The preset to apply.
        preset: ClusterPreset,
    },
    /// Choose the namespaces a cluster's session watches.
    #[serde(rename = "namespace::Select")]
    NamespaceSelect {
        /// The cluster whose selection changes.
        cluster: ClusterId,
        /// Namespace names; empty means all namespaces.
        namespaces: Vec<String>,
    },
    /// Add a namespace to a cluster's favourites, or remove it when it is one already.
    #[serde(rename = "namespace::ToggleFavourite")]
    NamespaceToggleFavourite {
        /// The cluster whose favourites change.
        cluster: ClusterId,
        /// The namespace to pin or unpin.
        namespace: String,
    },
    /// Add a kubeconfig source: a file, a directory, the default entry, or pasted text that
    /// Oxikube stores as its own file. Changes the settings list and local files, never a
    /// cluster.
    #[serde(rename = "kubeconfig::AddSource")]
    KubeconfigAddSource {
        /// What to add.
        source: NewKubeconfigSource,
    },
    /// Remove a kubeconfig source from the list. A file Oxikube stored itself (pasted) is
    /// deleted too; a file the user owns stays where it is.
    #[serde(rename = "kubeconfig::RemoveSource")]
    KubeconfigRemoveSource {
        /// Which source.
        source: KubeconfigSourceRef,
    },
    /// Re-read every kubeconfig source now.
    #[serde(rename = "kubeconfig::Reload")]
    KubeconfigReload,
    /// Open a registered view (overview, events, ...) by id.
    #[serde(rename = "view::Open")]
    ViewOpen {
        /// View id as registered by the owning ui crate.
        view: String,
    },
    /// Show or hide the command palette.
    #[serde(rename = "palette::Toggle")]
    PaletteToggle,
    /// Quit the application; asks first while operations are running.
    #[serde(rename = "app::Quit")]
    AppQuit,
    /// Open another main window.
    #[serde(rename = "window::New")]
    WindowNew,
    /// Make the UI one zoom step larger.
    #[serde(rename = "view::ZoomIn")]
    ViewZoomIn,
    /// Make the UI one zoom step smaller.
    #[serde(rename = "view::ZoomOut")]
    ViewZoomOut,
    /// Set the UI zoom back to 100 %.
    #[serde(rename = "view::ZoomReset")]
    ViewZoomReset,
    /// Copy the focused terminal's selection to the clipboard (E09-S06). Does nothing without a
    /// selection. Reads and changes nothing in a cluster.
    #[serde(rename = "terminal::Copy")]
    TerminalCopy,
    /// Paste the clipboard into the focused terminal (E09-S06), as a bracketed paste when the
    /// process asked for it; text with several lines asks first (setting
    /// `terminal.confirm_multiline_paste`). Writes only to the user's own session.
    #[serde(rename = "terminal::Paste")]
    TerminalPaste,
    /// Open a local shell in a new terminal (E09-S07): in the bottom dock of `cluster`'s tab (the
    /// displayed cluster tab when `None`) with that cluster's `KUBECONFIG`, context and namespace
    /// in its environment, or as a plain shell tab of the window when no cluster tab is shown.
    /// Starts a process on this machine; reads and changes nothing in a cluster.
    #[serde(rename = "terminal::New")]
    TerminalNew {
        /// The cluster whose tab gets the terminal; the displayed cluster tab when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
    },
    /// Open a new terminal in a new pane to the right of the active pane (E09-S07): the same
    /// shell, directory and cluster as the focused terminal, else a local shell of the displayed
    /// cluster. Reads and changes nothing in a cluster.
    #[serde(rename = "terminal::Split")]
    TerminalSplit,
    /// Close the focused terminal (E09-S07), ending its process; the active pane's terminal when
    /// focus is elsewhere. Does nothing when neither is a terminal.
    #[serde(rename = "terminal::Close")]
    TerminalClose,
    /// Open the focused pod terminal's session again after its connection dropped or it ended
    /// (E09-S12): a new session in the same container (the state of the old shell is gone). Goes
    /// through the exec service, which re-checks the exec capability. Does nothing for a terminal
    /// that is running or runs on this machine.
    #[serde(rename = "terminal::Reconnect")]
    TerminalReconnect,
    /// Start a new shell in the focused local terminal whose shell exited (E09-S12), with the same
    /// program, directory and cluster. Does nothing for a terminal that is running or runs in a
    /// pod.
    #[serde(rename = "terminal::Restart")]
    TerminalRestart,
    /// Open a link a terminal shows (cmd/ctrl-click, E09-S05): an `http`, `https`, `mailto` or
    /// `file` URL in the browser, or an absolute local path (an optional `:line[:column]` suffix
    /// is accepted) with the system's opener. Reads and changes nothing in a cluster.
    #[serde(rename = "terminal::OpenLink")]
    TerminalOpenLink {
        /// The URL or absolute path.
        target: String,
    },
    /// Open the list of the cluster's CustomResourceDefinitions (the sidebar's "Definitions").
    /// Read-only.
    #[serde(rename = "crd::OpenList")]
    CrdOpenList {
        /// Cluster whose definitions are listed.
        cluster: ClusterId,
    },
    /// Open the table of the custom resources the CRD `name` defines, for its served storage
    /// version (a CRD row's "Open", E07-S07). Read-only: the CRD is read to find the version.
    #[serde(rename = "crd::OpenResources")]
    CrdOpenResources {
        /// Cluster the CRD is in.
        cluster: ClusterId,
        /// The CRD's name (`widgets.example.com`).
        name: String,
    },
    /// Open the list view of a resource kind.
    #[serde(rename = "resource::OpenList")]
    ResourceOpenList {
        /// Cluster to list in.
        cluster: ClusterId,
        /// The kind to list.
        gvk: Gvk,
    },
    /// Open a resource's detail view.
    #[serde(rename = "resource::Open")]
    ResourceOpen {
        /// The resource to open.
        target: ResourceRef,
    },
    /// Copy a resource's name to the clipboard (a resource table's "Copy Name").
    #[serde(rename = "resource::CopyName")]
    ResourceCopyName {
        /// The resource whose name is copied.
        target: ResourceRef,
    },
    /// Restart the feed behind the open list views of a kind (the "Retry" button of a table that
    /// cannot list, E07-S10). Read-only: it reads the cluster again.
    #[serde(rename = "resource::RetryFeed")]
    ResourceRetryFeed {
        /// Cluster of the list.
        cluster: ClusterId,
        /// The kind listed.
        gvk: Gvk,
    },
    /// Pin a resource's detail drawer as a tab of the cluster's workspace (the drawer's "Pin as
    /// tab"). The drawer and the tab are one view, so nothing it shows is reset.
    #[serde(rename = "resource::PinDetail")]
    ResourcePinDetail {
        /// The resource whose detail is pinned.
        target: ResourceRef,
    },
    /// Copy one label (or annotation) of a resource to the clipboard as `key=value`. The value is
    /// read from the open detail, so it never travels in the command (nor into logs).
    #[serde(rename = "resource::CopyLabel")]
    ResourceCopyLabel {
        /// The resource the label is on.
        target: ResourceRef,
        /// The label or annotation key.
        key: String,
        /// Whether `key` names an annotation rather than a label.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        annotation: bool,
    },
    /// Copy the YAML the resource's open detail shows to the clipboard. The text is what the YAML
    /// tab displays (secret values masked, `managedFields` as the toggle has it), read from the
    /// open detail: it never travels in the command.
    #[serde(rename = "resource::CopyYaml")]
    ResourceCopyYaml {
        /// The resource whose YAML is copied.
        target: ResourceRef,
    },
    /// Save the YAML the resource's open detail shows to a file the user picks. Writes what is
    /// displayed, so a Secret's file holds the masked text.
    #[serde(rename = "resource::SaveYaml")]
    ResourceSaveYaml {
        /// The resource whose YAML is saved.
        target: ResourceRef,
    },
    /// Show or hide `metadata.managedFields` in the resource's YAML tab (hidden by default).
    #[serde(rename = "resource::ToggleManagedFields")]
    ResourceToggleManagedFields {
        /// The resource whose YAML tab is changed.
        target: ResourceRef,
    },
    /// Read the resource's describe text again (the Describe tab's refresh).
    #[serde(rename = "resource::RefreshDescribe")]
    ResourceRefreshDescribe {
        /// The resource described.
        target: ResourceRef,
    },
    /// Select every row of the open list views of a kind (`cmd-a` in a resource table).
    #[serde(rename = "resource::SelectAll")]
    ResourceSelectAll {
        /// Cluster of the list.
        cluster: ClusterId,
        /// The kind listed.
        gvk: Gvk,
    },
    /// Focus the filter bar of a kind's resource table (`/` in a table): the next keys type a
    /// filter (`/text`, `/!text`, `/-l selector`, `/-f fuzzy`).
    #[serde(rename = "table::FocusFilter")]
    TableFocusFilter {
        /// Cluster of the list.
        cluster: ClusterId,
        /// The kind listed.
        gvk: Gvk,
    },
    /// Open a resource's YAML.
    #[serde(rename = "resource::ViewYaml")]
    ResourceViewYaml {
        /// The resource to show.
        target: ResourceRef,
    },
    /// Delete any resource.
    #[serde(rename = "resource::Delete")]
    ResourceDelete {
        /// The resource to delete.
        target: ResourceRef,
        /// Dependent handling.
        #[serde(default)]
        propagation: Propagation,
    },
    /// Apply a manifest (server-side apply after a dry-run diff).
    #[serde(rename = "resource::Apply")]
    ResourceApply {
        /// Cluster to apply to.
        cluster: ClusterId,
        /// Default namespace for namespaced objects without one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        /// The YAML or JSON manifest text. May hold secrets: audit and logs
        /// must redact it.
        manifest: String,
    },
    /// Delete one pod.
    #[serde(rename = "pod::Delete")]
    PodDelete {
        /// The pod to delete.
        target: ResourceRef,
        /// Grace period override in seconds; `None` uses the pod's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grace_period_seconds: Option<u32>,
    },
    /// Open an interactive shell in a container: `bash`, else `sh` (the `terminal.exec_shells`
    /// chain), probed with a quick exec. An exec-class command: blocked on a read-only cluster
    /// unless `exec_in_read_only` allows it, never confirmed, audited on every open.
    #[serde(rename = "pod::Shell")]
    PodShell {
        /// The pod to open a shell in.
        target: ResourceRef,
        /// Container name; `None` picks the default one (the `kubectl.kubernetes.io/
        /// default-container` annotation, else the first container).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
    },
    /// Attach to the main process of a container (`kubectl attach -it`). An exec-class command
    /// like [`Command::PodShell`].
    #[serde(rename = "pod::Attach")]
    PodAttach {
        /// The pod to attach to.
        target: ResourceRef,
        /// Container name; `None` picks the default one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
    },
    /// Run a command (or an interactive shell) in a container. An exec-class command like
    /// [`Command::PodShell`].
    #[serde(rename = "pod::Exec")]
    PodExec {
        /// The pod to exec into.
        target: ResourceRef,
        /// Container name; `None` picks the default container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
        /// Argv to run; empty means an interactive shell.
        #[serde(default)]
        command: Vec<String>,
    },
    /// Forward a local port to a pod port.
    #[serde(rename = "pod::PortForward")]
    PodPortForward {
        /// The pod to forward to.
        target: ResourceRef,
        /// Local port; `None` picks a free one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_port: Option<u16>,
        /// The pod's port.
        remote_port: u16,
    },
    /// Open a pod's logs.
    #[serde(rename = "pod::ViewLogs")]
    PodViewLogs {
        /// The pod to read.
        target: ResourceRef,
        /// Container name; `None` means the default container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
        /// Keep streaming new lines.
        #[serde(default)]
        follow: bool,
        /// Show the previous container instance's logs.
        #[serde(default)]
        previous: bool,
        /// Only the last N lines.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail_lines: Option<u32>,
    },
    /// Empty a log view's local buffer (the cluster's logs are untouched; streaming continues).
    /// Asks first when the view has marked lines.
    #[serde(rename = "logs::Clear")]
    LogsClear {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Copy the selected lines of a log view to the clipboard; with no selection, the lines on
    /// screen.
    #[serde(rename = "logs::Copy")]
    LogsCopy {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Queue the selected lines of a log view (with no selection, the lines on screen) as context
    /// for the hosted agent, with where they came from. The agent panel (E27) drains the queue;
    /// until then it waits locally. Reads nothing from the cluster and changes nothing in it.
    #[serde(rename = "logs::SendToAgent")]
    LogsSendToAgent {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Run `kubectl logs -f` for what a log view shows (its pod and container, or a workload's pods
    /// by selector) in a terminal tab of the cluster, with the cluster's kubeconfig in the
    /// terminal's environment: the escape hatch for users who want kubectl itself. Starts a
    /// process on this machine; `kubectl logs` reads and changes nothing in the cluster. Does
    /// nothing when kubectl is not installed.
    #[serde(rename = "logs::TailInTerminal")]
    LogsTailInTerminal {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Mark or unmark the focused line of a log view.
    #[serde(rename = "logs::Mark")]
    LogsMark {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Switch a log view whose pod was replaced (deleted by a rollout, recreated by its
    /// StatefulSet) to the pod that took over, found through the gone pod's controller. Reads
    /// only; the view says when there is no replacement (yet).
    #[serde(rename = "logs::FollowReplacement")]
    LogsFollowReplacement {
        /// The object the log view shows (the gone pod).
        target: ResourceRef,
    },
    /// Open a log view's stream again after it failed (the reconnects ran out, a denied read
    /// that was fixed) or ended; the lines it holds stay and the overlap is not shown twice.
    #[serde(rename = "logs::Reconnect")]
    LogsReconnect {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Offer to save a log view's lines to a file: the view shows what would be written and the
    /// user picks the file, so nothing is written without them.
    #[serde(rename = "logs::Save")]
    LogsSave {
        /// The object the log view shows.
        target: ResourceRef,
        /// Which lines: the ones on screen, or the whole buffer.
        scope: LogSaveScope,
    },
    /// Read another part of a log view's log: the tail, the head or the last minutes
    /// (reopens the stream). `target` is what the view was opened on (a pod).
    #[serde(rename = "logs::SetRange")]
    LogsSetRange {
        /// The object the log view shows.
        target: ResourceRef,
        /// The part of the log to read.
        range: LogRange,
    },
    /// Show another container (regular, init or ephemeral) of a log view's pod (reopens the
    /// stream).
    #[serde(rename = "logs::SelectContainer")]
    LogsSelectContainer {
        /// The object the log view shows.
        target: ResourceRef,
        /// The container's name.
        container: String,
    },
    /// Follow a log view's newest line, or stop following it.
    #[serde(rename = "logs::ToggleAutoscroll")]
    LogsToggleAutoscroll {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Let a log view fill its cluster tab, or give the space back.
    #[serde(rename = "logs::ToggleFullscreen")]
    LogsToggleFullscreen {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Read the previous (terminated) instance of a log view's container, or the current one
    /// (reopens the stream).
    #[serde(rename = "logs::TogglePrevious")]
    LogsTogglePrevious {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Show or hide a log view's server timestamps.
    #[serde(rename = "logs::ToggleTimestamps")]
    LogsToggleTimestamps {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Show or hide the lines of one pod (or one of its containers) in a multi-pod log view.
    /// The source keeps streaming while hidden.
    #[serde(rename = "logs::ToggleSource")]
    LogsToggleSource {
        /// The workload or Service the log view shows.
        target: ResourceRef,
        /// The pod's name.
        pod: String,
        /// The container's name; `None` toggles every container of the pod.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
    },
    /// Wrap a log view's long lines, or let them run off the edge.
    #[serde(rename = "logs::ToggleWrap")]
    LogsToggleWrap {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Open a log view's search bar, optionally with `pattern` already typed.
    #[serde(rename = "logs::Find")]
    LogsFind {
        /// The object the log view shows.
        target: ResourceRef,
        /// The regular expression to search for; `None` just opens the bar.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    /// Go to the next match of a log view's search, wrapping from the last to the first.
    #[serde(rename = "logs::NextMatch")]
    LogsNextMatch {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Go to the previous match of a log view's search, wrapping from the first to the last.
    #[serde(rename = "logs::PreviousMatch")]
    LogsPreviousMatch {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Make a log view's search case-sensitive, or case-insensitive again.
    #[serde(rename = "logs::ToggleCase")]
    LogsToggleCase {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Match the lines that do not contain a log view's pattern, or those that do.
    #[serde(rename = "logs::ToggleInverse")]
    LogsToggleInverse {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Hide the lines that do not match a log view's search, or show every line with the matches
    /// highlighted.
    #[serde(rename = "logs::ToggleFilterMode")]
    LogsToggleFilterMode {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Close a log view's search bar and clear its highlights and filter.
    #[serde(rename = "logs::CloseSearch")]
    LogsCloseSearch {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Show structured (JSON) lines as level, time and message columns with expandable fields,
    /// or show every line as the raw text it is.
    #[serde(rename = "logs::ToggleJsonMode")]
    LogsToggleJsonMode {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Show or hide the lines of one level in a log view (the level chips).
    #[serde(rename = "logs::ToggleLevel")]
    LogsToggleLevel {
        /// The object the log view shows.
        target: ResourceRef,
        /// The chip to flip: a level, or `text` for plain and level-less lines.
        level: LevelChip,
    },
    /// Expand a structured line of a log view into its pretty-printed pane, or close the pane
    /// when it already shows that line.
    #[serde(rename = "logs::ToggleLine")]
    LogsToggleLine {
        /// The object the log view shows.
        target: ResourceRef,
        /// The line's sequence number in the view (its row's `seq`).
        seq: u64,
    },
    /// Close the expanded-line pane of a log view.
    #[serde(rename = "logs::CollapseLine")]
    LogsCollapseLine {
        /// The object the log view shows.
        target: ResourceRef,
    },
    /// Set a workload's replica count.
    #[serde(rename = "workload::Scale")]
    WorkloadScale {
        /// The Deployment, StatefulSet or ReplicaSet to scale.
        target: ResourceRef,
        /// Desired replicas.
        replicas: u32,
    },
    /// Rolling-restart a workload.
    #[serde(rename = "workload::Restart")]
    WorkloadRestart {
        /// The workload to restart.
        target: ResourceRef,
    },
    /// Open the logs of every pod of a workload or Service merged by time, one colour per pod
    /// (stern-style). The pods are the ones the object's selector matches.
    #[serde(rename = "workload::ViewLogs")]
    WorkloadViewLogs {
        /// The Deployment, StatefulSet, DaemonSet, ReplicaSet, Job or Service to read.
        target: ResourceRef,
        /// A further label selector (`app=web,tier!=db`) the pods must match too.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selector: Option<String>,
        /// Read only the containers of this name; `None` reads every container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        container: Option<String>,
        /// Keep streaming new lines (default `true`).
        #[serde(default = "default_follow")]
        follow: bool,
        /// Only the last N lines of every pod.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tail_lines: Option<u32>,
    },
    /// Mark a node unschedulable.
    #[serde(rename = "node::Cordon")]
    NodeCordon {
        /// The node.
        target: ResourceRef,
    },
    /// Mark a node schedulable again.
    #[serde(rename = "node::Uncordon")]
    NodeUncordon {
        /// The node.
        target: ResourceRef,
    },
    /// Cordon a node and evict its pods.
    #[serde(rename = "node::Drain")]
    NodeDrain {
        /// The node.
        target: ResourceRef,
        /// Also evict pods not managed by a controller.
        #[serde(default)]
        force: bool,
    },
}

impl Command {
    /// The command's id (keymap action name, serde `type` tag, tool name source).
    pub const fn id(&self) -> CommandId {
        match self {
            Command::ClusterConnect { .. } => CommandId::CLUSTER_CONNECT,
            Command::ClusterCancelConnect { .. } => CommandId::CLUSTER_CANCEL_CONNECT,
            Command::ClusterCloseTab { .. } => CommandId::CLUSTER_CLOSE_TAB,
            Command::ClusterDisconnect { .. } => CommandId::CLUSTER_DISCONNECT,
            Command::ClusterReconnect { .. } => CommandId::CLUSTER_RECONNECT,
            Command::ClusterNextTab => CommandId::CLUSTER_NEXT_TAB,
            Command::ClusterPreviousTab => CommandId::CLUSTER_PREVIOUS_TAB,
            Command::ClusterSelect { .. } => CommandId::CLUSTER_SELECT,
            Command::ClusterSwitchTab { .. } => CommandId::CLUSTER_SWITCH_TAB,
            Command::ClusterToggleFavourite { .. } => CommandId::CLUSTER_TOGGLE_FAVOURITE,
            Command::ClusterToggleReadOnly { .. } => CommandId::CLUSTER_TOGGLE_READ_ONLY,
            Command::ClusterSetColour { .. } => CommandId::CLUSTER_SET_COLOUR,
            Command::ClusterApplyPreset { .. } => CommandId::CLUSTER_APPLY_PRESET,
            Command::NamespaceSelect { .. } => CommandId::NAMESPACE_SELECT,
            Command::NamespaceToggleFavourite { .. } => CommandId::NAMESPACE_TOGGLE_FAVOURITE,
            Command::KubeconfigAddSource { .. } => CommandId::KUBECONFIG_ADD_SOURCE,
            Command::KubeconfigRemoveSource { .. } => CommandId::KUBECONFIG_REMOVE_SOURCE,
            Command::KubeconfigReload => CommandId::KUBECONFIG_RELOAD,
            Command::ViewOpen { .. } => CommandId::VIEW_OPEN,
            Command::PaletteToggle => CommandId::PALETTE_TOGGLE,
            Command::AppQuit => CommandId::APP_QUIT,
            Command::WindowNew => CommandId::WINDOW_NEW,
            Command::ViewZoomIn => CommandId::VIEW_ZOOM_IN,
            Command::ViewZoomOut => CommandId::VIEW_ZOOM_OUT,
            Command::ViewZoomReset => CommandId::VIEW_ZOOM_RESET,
            Command::TerminalOpenLink { .. } => CommandId::TERMINAL_OPEN_LINK,
            Command::TerminalCopy => CommandId::TERMINAL_COPY,
            Command::TerminalPaste => CommandId::TERMINAL_PASTE,
            Command::TerminalNew { .. } => CommandId::TERMINAL_NEW,
            Command::TerminalSplit => CommandId::TERMINAL_SPLIT,
            Command::TerminalClose => CommandId::TERMINAL_CLOSE,
            Command::TerminalReconnect => CommandId::TERMINAL_RECONNECT,
            Command::TerminalRestart => CommandId::TERMINAL_RESTART,
            Command::CrdOpenList { .. } => CommandId::CRD_OPEN_LIST,
            Command::CrdOpenResources { .. } => CommandId::CRD_OPEN_RESOURCES,
            Command::ResourceOpenList { .. } => CommandId::RESOURCE_OPEN_LIST,
            Command::ResourceOpen { .. } => CommandId::RESOURCE_OPEN,
            Command::ResourceCopyName { .. } => CommandId::RESOURCE_COPY_NAME,
            Command::ResourceRetryFeed { .. } => CommandId::RESOURCE_RETRY_FEED,
            Command::ResourcePinDetail { .. } => CommandId::RESOURCE_PIN_DETAIL,
            Command::ResourceCopyLabel { .. } => CommandId::RESOURCE_COPY_LABEL,
            Command::ResourceCopyYaml { .. } => CommandId::RESOURCE_COPY_YAML,
            Command::ResourceSaveYaml { .. } => CommandId::RESOURCE_SAVE_YAML,
            Command::ResourceToggleManagedFields { .. } => {
                CommandId::RESOURCE_TOGGLE_MANAGED_FIELDS
            }
            Command::ResourceRefreshDescribe { .. } => CommandId::RESOURCE_REFRESH_DESCRIBE,
            Command::ResourceSelectAll { .. } => CommandId::RESOURCE_SELECT_ALL,
            Command::TableFocusFilter { .. } => CommandId::TABLE_FOCUS_FILTER,
            Command::ResourceViewYaml { .. } => CommandId::RESOURCE_VIEW_YAML,
            Command::ResourceDelete { .. } => CommandId::RESOURCE_DELETE,
            Command::ResourceApply { .. } => CommandId::RESOURCE_APPLY,
            Command::PodDelete { .. } => CommandId::POD_DELETE,
            Command::PodShell { .. } => CommandId::POD_SHELL,
            Command::PodAttach { .. } => CommandId::POD_ATTACH,
            Command::PodExec { .. } => CommandId::POD_EXEC,
            Command::PodPortForward { .. } => CommandId::POD_PORT_FORWARD,
            Command::PodViewLogs { .. } => CommandId::POD_VIEW_LOGS,
            Command::LogsClear { .. } => CommandId::LOGS_CLEAR,
            Command::LogsCopy { .. } => CommandId::LOGS_COPY,
            Command::LogsMark { .. } => CommandId::LOGS_MARK,
            Command::LogsSendToAgent { .. } => CommandId::LOGS_SEND_TO_AGENT,
            Command::LogsTailInTerminal { .. } => CommandId::LOGS_TAIL_IN_TERMINAL,
            Command::LogsFollowReplacement { .. } => CommandId::LOGS_FOLLOW_REPLACEMENT,
            Command::LogsReconnect { .. } => CommandId::LOGS_RECONNECT,
            Command::LogsSave { .. } => CommandId::LOGS_SAVE,
            Command::LogsSetRange { .. } => CommandId::LOGS_SET_RANGE,
            Command::LogsSelectContainer { .. } => CommandId::LOGS_SELECT_CONTAINER,
            Command::LogsToggleAutoscroll { .. } => CommandId::LOGS_TOGGLE_AUTOSCROLL,
            Command::LogsToggleFullscreen { .. } => CommandId::LOGS_TOGGLE_FULLSCREEN,
            Command::LogsTogglePrevious { .. } => CommandId::LOGS_TOGGLE_PREVIOUS,
            Command::LogsToggleTimestamps { .. } => CommandId::LOGS_TOGGLE_TIMESTAMPS,
            Command::LogsToggleSource { .. } => CommandId::LOGS_TOGGLE_SOURCE,
            Command::LogsToggleWrap { .. } => CommandId::LOGS_TOGGLE_WRAP,
            Command::LogsFind { .. } => CommandId::LOGS_FIND,
            Command::LogsNextMatch { .. } => CommandId::LOGS_NEXT_MATCH,
            Command::LogsPreviousMatch { .. } => CommandId::LOGS_PREVIOUS_MATCH,
            Command::LogsToggleCase { .. } => CommandId::LOGS_TOGGLE_CASE,
            Command::LogsToggleInverse { .. } => CommandId::LOGS_TOGGLE_INVERSE,
            Command::LogsToggleFilterMode { .. } => CommandId::LOGS_TOGGLE_FILTER_MODE,
            Command::LogsCloseSearch { .. } => CommandId::LOGS_CLOSE_SEARCH,
            Command::LogsToggleJsonMode { .. } => CommandId::LOGS_TOGGLE_JSON_MODE,
            Command::LogsToggleLevel { .. } => CommandId::LOGS_TOGGLE_LEVEL,
            Command::LogsToggleLine { .. } => CommandId::LOGS_TOGGLE_LINE,
            Command::LogsCollapseLine { .. } => CommandId::LOGS_COLLAPSE_LINE,
            Command::WorkloadScale { .. } => CommandId::WORKLOAD_SCALE,
            Command::WorkloadRestart { .. } => CommandId::WORKLOAD_RESTART,
            Command::WorkloadViewLogs { .. } => CommandId::WORKLOAD_VIEW_LOGS,
            Command::NodeCordon { .. } => CommandId::NODE_CORDON,
            Command::NodeUncordon { .. } => CommandId::NODE_UNCORDON,
            Command::NodeDrain { .. } => CommandId::NODE_DRAIN,
        }
    }

    /// The command's static metadata.
    ///
    /// # Panics
    ///
    /// Panics if the registry lacks this variant's id; the registry tests make
    /// that impossible in a green build.
    pub fn meta(&self) -> &'static CommandMeta {
        registry::lookup(self.id()).expect("every Command variant has a registry entry")
    }

    /// Whether this command goes through `MutationGuard`.
    pub fn is_mutating(&self) -> bool {
        self.meta().mutating
    }

    /// Whether this command opens an interactive session in a container (the exec class: see
    /// [`CommandMeta::exec`]).
    pub fn is_exec(&self) -> bool {
        self.meta().exec
    }

    /// The resource this command targets, if it acts on a single selection.
    pub fn target(&self) -> Option<&ResourceRef> {
        match self {
            Command::ResourceOpen { target }
            | Command::ResourceCopyName { target }
            | Command::ResourcePinDetail { target }
            | Command::ResourceCopyLabel { target, .. }
            | Command::ResourceCopyYaml { target }
            | Command::ResourceSaveYaml { target }
            | Command::ResourceToggleManagedFields { target }
            | Command::ResourceRefreshDescribe { target }
            | Command::ResourceViewYaml { target }
            | Command::ResourceDelete { target, .. }
            | Command::PodDelete { target, .. }
            | Command::PodShell { target, .. }
            | Command::PodAttach { target, .. }
            | Command::PodExec { target, .. }
            | Command::PodPortForward { target, .. }
            | Command::PodViewLogs { target, .. }
            | Command::LogsClear { target }
            | Command::LogsCopy { target }
            | Command::LogsMark { target }
            | Command::LogsSendToAgent { target }
            | Command::LogsTailInTerminal { target }
            | Command::LogsFollowReplacement { target }
            | Command::LogsReconnect { target }
            | Command::LogsSave { target, .. }
            | Command::LogsSetRange { target, .. }
            | Command::LogsSelectContainer { target, .. }
            | Command::LogsToggleAutoscroll { target }
            | Command::LogsToggleFullscreen { target }
            | Command::LogsTogglePrevious { target }
            | Command::LogsToggleTimestamps { target }
            | Command::LogsToggleSource { target, .. }
            | Command::LogsToggleWrap { target }
            | Command::LogsFind { target, .. }
            | Command::LogsNextMatch { target }
            | Command::LogsPreviousMatch { target }
            | Command::LogsToggleCase { target }
            | Command::LogsToggleInverse { target }
            | Command::LogsToggleFilterMode { target }
            | Command::LogsCloseSearch { target }
            | Command::LogsToggleJsonMode { target }
            | Command::LogsToggleLevel { target, .. }
            | Command::LogsToggleLine { target, .. }
            | Command::LogsCollapseLine { target }
            | Command::WorkloadScale { target, .. }
            | Command::WorkloadRestart { target }
            | Command::WorkloadViewLogs { target, .. }
            | Command::NodeCordon { target }
            | Command::NodeUncordon { target }
            | Command::NodeDrain { target, .. } => Some(target),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::command::{COMMANDS, PastedText};
    use crate::ids::ContextName;

    fn cluster() -> ClusterId {
        ClusterId::new("~/.kube/config", &ContextName::new("kind-oxikube"))
    }

    fn pod() -> ResourceRef {
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "default", "web-0")
    }

    fn deployment() -> ResourceRef {
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("apps", "v1", "Deployment"),
            "prod",
            "api",
        )
    }

    fn node() -> ResourceRef {
        ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "worker-1")
    }

    /// One sample of every variant.
    fn samples() -> Vec<Command> {
        vec![
            Command::ClusterConnect { cluster: cluster() },
            Command::ClusterCancelConnect { cluster: cluster() },
            Command::ClusterCloseTab { cluster: cluster() },
            Command::ClusterDisconnect { cluster: cluster() },
            Command::ClusterReconnect { cluster: cluster() },
            Command::ClusterNextTab,
            Command::ClusterPreviousTab,
            Command::ClusterSelect { cluster: cluster() },
            Command::ClusterSwitchTab { index: 2 },
            Command::ClusterToggleFavourite {
                cluster: cluster(),
                favourite: None,
            },
            Command::ClusterToggleFavourite {
                cluster: cluster(),
                favourite: Some(true),
            },
            Command::ClusterToggleReadOnly {
                cluster: cluster(),
                read_only: None,
            },
            Command::ClusterToggleReadOnly {
                cluster: cluster(),
                read_only: Some(true),
            },
            Command::ClusterSetColour {
                cluster: cluster(),
                colour: Some(ClusterColour::rgb(0xe5, 0x48, 0x4d)),
            },
            Command::ClusterSetColour {
                cluster: cluster(),
                colour: None,
            },
            Command::ClusterApplyPreset {
                cluster: cluster(),
                preset: ClusterPreset::Prod,
            },
            Command::NamespaceSelect {
                cluster: cluster(),
                namespaces: vec!["default".into(), "kube-system".into()],
            },
            Command::NamespaceToggleFavourite {
                cluster: cluster(),
                namespace: "kube-system".into(),
            },
            Command::KubeconfigAddSource {
                source: NewKubeconfigSource::File {
                    path: "/work/prod.yaml".into(),
                },
            },
            Command::KubeconfigAddSource {
                source: NewKubeconfigSource::Pasted {
                    name: "prod".into(),
                    text: PastedText::new("apiVersion: v1\nkind: Config\n"),
                },
            },
            Command::KubeconfigRemoveSource {
                source: KubeconfigSourceRef::Dir {
                    path: "/work/configs".into(),
                },
            },
            Command::KubeconfigReload,
            Command::ViewOpen {
                view: "overview".into(),
            },
            Command::PaletteToggle,
            Command::AppQuit,
            Command::WindowNew,
            Command::ViewZoomIn,
            Command::ViewZoomOut,
            Command::ViewZoomReset,
            Command::TerminalOpenLink {
                target: "https://kubernetes.io".into(),
            },
            Command::TerminalCopy,
            Command::TerminalPaste,
            Command::TerminalNew { cluster: None },
            Command::TerminalNew {
                cluster: Some(cluster()),
            },
            Command::TerminalSplit,
            Command::TerminalClose,
            Command::TerminalReconnect,
            Command::TerminalRestart,
            Command::CrdOpenList { cluster: cluster() },
            Command::CrdOpenResources {
                cluster: cluster(),
                name: "widgets.example.com".into(),
            },
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk: Gvk::new("apps", "v1", "Deployment"),
            },
            Command::ResourceOpen { target: pod() },
            Command::ResourceCopyName { target: pod() },
            Command::ResourceRetryFeed {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::ResourcePinDetail { target: pod() },
            Command::ResourceCopyLabel {
                target: pod(),
                key: "app".into(),
                annotation: false,
            },
            Command::ResourceCopyYaml { target: pod() },
            Command::ResourceSaveYaml { target: pod() },
            Command::ResourceToggleManagedFields { target: pod() },
            Command::ResourceRefreshDescribe { target: pod() },
            Command::ResourceSelectAll {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::TableFocusFilter {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::ResourceViewYaml { target: pod() },
            Command::ResourceDelete {
                target: deployment(),
                propagation: Propagation::Foreground,
            },
            Command::ResourceApply {
                cluster: cluster(),
                namespace: Some("default".into()),
                manifest: "kind: ConfigMap".into(),
            },
            Command::PodDelete {
                target: pod(),
                grace_period_seconds: Some(0),
            },
            Command::PodShell {
                target: pod(),
                container: Some("app".into()),
            },
            Command::PodAttach {
                target: pod(),
                container: None,
            },
            Command::PodExec {
                target: pod(),
                container: Some("app".into()),
                command: vec!["sh".into(), "-c".into(), "id".into()],
            },
            Command::PodPortForward {
                target: pod(),
                local_port: None,
                remote_port: 8080,
            },
            Command::PodViewLogs {
                target: pod(),
                container: None,
                follow: true,
                previous: false,
                tail_lines: Some(500),
            },
            Command::LogsClear { target: pod() },
            Command::LogsCopy { target: pod() },
            Command::LogsMark { target: pod() },
            Command::LogsSendToAgent { target: pod() },
            Command::LogsTailInTerminal { target: pod() },
            Command::LogsFollowReplacement { target: pod() },
            Command::LogsReconnect { target: pod() },
            Command::LogsSave {
                target: pod(),
                scope: LogSaveScope::All,
            },
            Command::LogsSetRange {
                target: pod(),
                range: LogRange::Last15m,
            },
            Command::LogsSelectContainer {
                target: pod(),
                container: "init-db".into(),
            },
            Command::LogsToggleAutoscroll { target: pod() },
            Command::LogsToggleFullscreen { target: pod() },
            Command::LogsTogglePrevious { target: pod() },
            Command::LogsToggleTimestamps { target: pod() },
            Command::LogsToggleSource {
                target: deployment(),
                pod: "web-7d9".into(),
                container: Some("app".into()),
            },
            Command::LogsToggleWrap { target: pod() },
            Command::LogsFind {
                target: pod(),
                pattern: Some("timeout".into()),
            },
            Command::LogsNextMatch { target: pod() },
            Command::LogsPreviousMatch { target: pod() },
            Command::LogsToggleCase { target: pod() },
            Command::LogsToggleInverse { target: pod() },
            Command::LogsToggleFilterMode { target: pod() },
            Command::LogsCloseSearch { target: pod() },
            Command::LogsToggleJsonMode { target: pod() },
            Command::LogsToggleLevel {
                target: pod(),
                level: LevelChip::Warn,
            },
            Command::LogsToggleLine {
                target: pod(),
                seq: 7,
            },
            Command::LogsCollapseLine { target: pod() },
            Command::WorkloadViewLogs {
                target: deployment(),
                selector: Some("tier=api".into()),
                container: None,
                follow: true,
                tail_lines: Some(200),
            },
            Command::WorkloadScale {
                target: deployment(),
                replicas: 3,
            },
            Command::WorkloadRestart {
                target: deployment(),
            },
            Command::NodeCordon { target: node() },
            Command::NodeUncordon { target: node() },
            Command::NodeDrain {
                target: node(),
                force: true,
            },
        ]
    }

    #[test]
    fn json_round_trip_for_every_variant() {
        for command in samples() {
            let json = serde_json::to_value(&command).unwrap();
            assert_eq!(
                json["type"],
                command.id().as_str(),
                "type tag must equal the command id"
            );
            let back: Command = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(back, command, "{json}");
            let text = serde_json::to_string(&command).unwrap();
            assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), command);
        }
    }

    #[test]
    fn samples_cover_exactly_the_registry() {
        let from_samples: BTreeSet<_> = samples().iter().map(Command::id).collect();
        let from_registry: BTreeSet<_> = COMMANDS.iter().map(|m| m.id).collect();
        assert_eq!(from_samples, from_registry);
        for command in samples() {
            assert_eq!(command.meta().id, command.id());
        }
    }

    #[test]
    fn tool_call_arguments_become_a_command() {
        // An MCP `k8s.workload_scale` call: the tool name picks the id, the
        // arguments are the payload.
        let args = json!({ "target": deployment(), "replicas": 5 });
        let id = COMMANDS
            .iter()
            .find(|m| m.id.tool_name() == "k8s.workload_scale")
            .unwrap()
            .id;
        let mut value = args;
        value["type"] = json!(id.as_str());
        let command: Command = serde_json::from_value(value).unwrap();
        assert_eq!(
            command,
            Command::WorkloadScale {
                target: deployment(),
                replicas: 5
            }
        );
        assert!(command.is_mutating());
        assert_eq!(command.target(), Some(&deployment()));
    }

    #[test]
    fn exec_commands_parse_from_tool_arguments_and_are_exec_class() {
        let shell: Command = serde_json::from_value(json!({
            "type": "pod::Shell",
            "target": pod(),
        }))
        .unwrap();
        assert_eq!(
            shell,
            Command::PodShell {
                target: pod(),
                container: None
            }
        );
        assert!(shell.is_exec() && !shell.is_mutating());
        assert_eq!(shell.effective_risk(), None, "a shell is not a mutation");
        for command in samples() {
            let exec = matches!(
                command,
                Command::PodShell { .. } | Command::PodAttach { .. } | Command::PodExec { .. }
            );
            assert_eq!(command.is_exec(), exec, "{}", command.id());
        }
    }

    #[test]
    fn optional_fields_default() {
        let delete: Command = serde_json::from_value(json!({
            "type": "resource::Delete",
            "target": deployment(),
        }))
        .unwrap();
        assert_eq!(
            delete,
            Command::ResourceDelete {
                target: deployment(),
                propagation: Propagation::Background
            }
        );
        let logs: Command = serde_json::from_value(json!({
            "type": "pod::ViewLogs",
            "target": pod(),
        }))
        .unwrap();
        assert!(matches!(
            logs,
            Command::PodViewLogs {
                follow: false,
                previous: false,
                container: None,
                tail_lines: None,
                ..
            }
        ));
        let unit: Command = serde_json::from_value(json!({ "type": "palette::Toggle" })).unwrap();
        assert_eq!(unit, Command::PaletteToggle);
    }

    #[test]
    fn rejects_unknown_or_missing_type() {
        assert!(serde_json::from_value::<Command>(json!({ "type": "pod::Explode" })).is_err());
        assert!(serde_json::from_value::<Command>(json!({ "replicas": 3 })).is_err());
        assert!(
            serde_json::from_value::<Command>(
                json!({ "type": "workload::Scale", "target": pod() })
            )
            .is_err()
        );
    }

    #[test]
    fn navigation_commands_do_not_mutate() {
        for command in samples() {
            if matches!(
                command,
                Command::ClusterConnect { .. }
                    | Command::ClusterCloseTab { .. }
                    | Command::ClusterCancelConnect { .. }
                    | Command::ClusterReconnect { .. }
                    | Command::ClusterNextTab
                    | Command::ClusterPreviousTab
                    | Command::ClusterSwitchTab { .. }
                    | Command::ClusterDisconnect { .. }
                    | Command::ClusterToggleFavourite { .. }
                    | Command::ClusterSelect { .. }
                    | Command::NamespaceSelect { .. }
                    | Command::NamespaceToggleFavourite { .. }
                    | Command::ViewOpen { .. }
                    | Command::PaletteToggle
                    | Command::AppQuit
                    | Command::WindowNew
                    | Command::ViewZoomIn
                    | Command::ViewZoomOut
                    | Command::ViewZoomReset
                    | Command::TerminalOpenLink { .. }
                    | Command::TerminalCopy
                    | Command::TerminalPaste
                    | Command::TerminalNew { .. }
                    | Command::TerminalSplit
                    | Command::TerminalClose
                    | Command::TerminalReconnect
                    | Command::TerminalRestart
                    | Command::CrdOpenList { .. }
                    | Command::CrdOpenResources { .. }
                    | Command::ResourceOpen { .. }
                    | Command::ResourceOpenList { .. }
                    | Command::ResourceCopyName { .. }
                    | Command::ResourceRetryFeed { .. }
                    | Command::ResourcePinDetail { .. }
                    | Command::ResourceCopyLabel { .. }
                    | Command::ResourceCopyYaml { .. }
                    | Command::ResourceSaveYaml { .. }
                    | Command::ResourceToggleManagedFields { .. }
                    | Command::ResourceRefreshDescribe { .. }
                    | Command::ResourceSelectAll { .. }
                    | Command::TableFocusFilter { .. }
                    | Command::ResourceViewYaml { .. }
                    | Command::LogsClear { .. }
                    | Command::LogsCopy { .. }
                    | Command::LogsMark { .. }
                    | Command::LogsSendToAgent { .. }
                    | Command::LogsTailInTerminal { .. }
                    | Command::LogsFollowReplacement { .. }
                    | Command::LogsReconnect { .. }
                    | Command::LogsSave { .. }
                    | Command::LogsSetRange { .. }
                    | Command::LogsSelectContainer { .. }
                    | Command::LogsToggleAutoscroll { .. }
                    | Command::LogsToggleFullscreen { .. }
                    | Command::LogsTogglePrevious { .. }
                    | Command::LogsToggleTimestamps { .. }
                    | Command::LogsToggleSource { .. }
                    | Command::LogsToggleWrap { .. }
                    | Command::LogsFind { .. }
                    | Command::LogsNextMatch { .. }
                    | Command::LogsPreviousMatch { .. }
                    | Command::LogsToggleCase { .. }
                    | Command::LogsToggleInverse { .. }
                    | Command::LogsToggleFilterMode { .. }
                    | Command::LogsCloseSearch { .. }
                    | Command::LogsToggleJsonMode { .. }
                    | Command::LogsToggleLevel { .. }
                    | Command::LogsToggleLine { .. }
                    | Command::LogsCollapseLine { .. }
                    | Command::WorkloadViewLogs { .. }
                    | Command::ClusterToggleReadOnly { .. }
                    | Command::ClusterSetColour { .. }
                    | Command::ClusterApplyPreset { .. }
            ) {
                assert!(!command.is_mutating(), "{}", command.id());
            }
        }
    }
}

#[cfg(test)]
mod kubeconfig_tests {
    use serde_json::json;

    use super::*;
    use crate::command::PastedText;

    #[test]
    fn kubeconfig_commands_use_flat_json_and_name_their_tools() {
        let add: Command = serde_json::from_value(json!({
            "type": "kubeconfig::AddSource",
            "source": { "kind": "dir", "path": "/work/configs" }
        }))
        .unwrap();
        assert_eq!(
            add,
            Command::KubeconfigAddSource {
                source: NewKubeconfigSource::Dir {
                    path: "/work/configs".into()
                }
            }
        );
        assert_eq!(
            add.id().tool_name(),
            "app.kubeconfig_add_source",
            "kubeconfig commands are app-level tools"
        );
        assert_eq!(
            Command::KubeconfigReload.id().tool_name(),
            "app.kubeconfig_reload"
        );
        assert_eq!(
            serde_json::to_value(Command::KubeconfigReload).unwrap(),
            json!({ "type": "kubeconfig::Reload" })
        );
    }

    #[test]
    fn pasted_text_travels_as_a_string_but_never_prints() {
        let secret = "token: s3cr3t-do-not-leak";
        let command = Command::KubeconfigAddSource {
            source: NewKubeconfigSource::Pasted {
                name: "prod".into(),
                text: PastedText::new(secret),
            },
        };
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["source"]["text"], secret);
        let again: Command = serde_json::from_value(json).unwrap();
        assert_eq!(again, command);
        let shown = format!("{command:?} {again:#?}");
        assert!(!shown.contains("s3cr3t"), "{shown}");
        assert!(shown.contains("bytes"), "{shown}");
    }

    #[test]
    fn the_kubeconfig_commands_are_read_class_without_a_guard_tier() {
        for command in [
            Command::KubeconfigReload,
            Command::KubeconfigAddSource {
                source: NewKubeconfigSource::Default,
            },
            Command::KubeconfigRemoveSource {
                source: KubeconfigSourceRef::Default,
            },
        ] {
            let meta = command.meta();
            assert!(!meta.mutating && !meta.privileged, "{}", meta.id);
            assert_eq!(meta.confirm, crate::safety::ConfirmTier::None);
        }
    }
}
