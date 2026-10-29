//! The [`Command`] payload enum: navigation plus resource verbs.

use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use super::cloud::CloudRefreshMode;
use super::id::CommandId;
use super::image::ImageChange;
use super::kubeconfig::{KubeconfigSourceRef, NewKubeconfigSource};
use super::meta::CommandMeta;
use super::registry;
use crate::audit::AuditExportFormat;
use crate::cloud::CloudProvider;
use crate::colour::ClusterColour;
use crate::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use crate::log::{LevelChip, LogRange, LogSaveScope};
use crate::portforward::ForwardPort;
use crate::preset::ClusterPreset;
use crate::selection::{LabelEdit, LabelField};

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

/// The image of a debug container when `pod::Debug` names none (`kubectl debug`'s own
/// suggestion for a minimal toolbox).
pub const DEFAULT_DEBUG_IMAGE: &str = "busybox";

/// The program a debug container runs when `pod::Debug` names none.
pub const DEFAULT_DEBUG_COMMAND: &str = "sh";

fn default_debug_image() -> String {
    DEFAULT_DEBUG_IMAGE.to_owned()
}

/// `true`: the serde default of flags that are on unless the caller says otherwise.
fn default_follow() -> bool {
    true
}

/// `true`: the serde default of `storageclass::SetDefault`'s `clear_previous`.
fn default_clear_previous() -> bool {
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
    /// Open the Helm repositories page (E16-S07): the repositories in Helm's own configuration,
    /// with add, remove and update. Reads only local state.
    #[serde(rename = "helm::ShowRepos")]
    HelmShowRepos,
    /// Open the cluster's Helm releases (E16-S05, `:helm`): the table of releases with name,
    /// namespace, chart, revision, version, app version, status and age, and a detail with the
    /// release's resources, values, manifest, notes and history. Reads the release Secrets and
    /// ConfigMaps natively, so it needs no `helm` binary.
    #[serde(rename = "helm::OpenReleases")]
    HelmOpenReleases {
        /// The cluster whose releases open.
        cluster: ClusterId,
    },
    /// Open the users page of the cluster's RBAC (E17-S07, `:usr`): one row per user and
    /// ServiceAccount named by any RoleBinding or ClusterRoleBinding, with how many bindings and
    /// roles reach it. Reads only the bindings and roles.
    #[serde(rename = "rbac::OpenUsers")]
    RbacOpenUsers {
        /// The cluster whose users open.
        cluster: ClusterId,
    },
    /// Open the groups page of the cluster's RBAC (E17-S07, `:grp`): one row per group named by
    /// any binding. The API cannot list a group's members, so the page lists none.
    #[serde(rename = "rbac::OpenGroups")]
    RbacOpenGroups {
        /// The cluster whose groups open.
        cluster: ClusterId,
    },
    /// Show what a subject may do (E17-S07, `:rbac <subject>`): a per-verb matrix by API group
    /// and resource, each cell traced to the bindings and roles that grant it. Reads only the
    /// bindings and roles.
    #[serde(rename = "rbac::ShowSubject")]
    RbacShowSubject {
        /// The cluster whose policy is read.
        cluster: ClusterId,
        /// The subject: a user name, `system:serviceaccount:<ns>:<name>`, or a group name.
        subject: String,
    },
    /// Copy the open subject matrix to the clipboard as tab-separated text (E17-S07), for a
    /// ticket.
    #[serde(rename = "rbac::CopyMatrix")]
    RbacCopyMatrix {
        /// The cluster whose matrix view to copy from.
        cluster: ClusterId,
    },
    /// Print a release's values (E16-S05), the user-supplied ones or with `all` the chart defaults
    /// merged in. Credential-looking values are masked (`********`) unless the user revealed
    /// them with `helm::RevealValues`. Reads only.
    #[serde(rename = "helm::ShowValues")]
    HelmShowValues {
        /// The release's cluster.
        cluster: ClusterId,
        /// The release's namespace.
        namespace: String,
        /// The release name.
        name: String,
        /// The revision; the latest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u32>,
        /// Computed values (chart defaults merged with the user's) instead of the user's only.
        #[serde(default)]
        all: bool,
    },
    /// Print a release's rendered manifest (E16-S05). Secret documents are masked unless the user
    /// revealed the release's values with `helm::RevealValues`. Reads only.
    #[serde(rename = "helm::ShowManifest")]
    HelmShowManifest {
        /// The release's cluster.
        cluster: ClusterId,
        /// The release's namespace.
        namespace: String,
        /// The release name.
        name: String,
        /// The revision; the latest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<u32>,
    },
    /// Show a release's masked values and manifest Secrets in clear in the release detail
    /// (E16-S05). The user's explicit action: the grant lasts while the detail is open and
    /// `reveal: false` hides the values again. The reply says how many values the view now shows
    /// and never carries them.
    #[serde(rename = "helm::RevealValues")]
    HelmRevealValues {
        /// The release's cluster.
        cluster: ClusterId,
        /// The release's namespace.
        namespace: String,
        /// The release name.
        name: String,
        /// `false` hides the values again (default: reveal).
        #[serde(default = "default_follow")]
        reveal: bool,
    },
    /// Open the audit log viewer (E19-S09) in the active cluster tab, or the one the cluster
    /// names: every guarded mutation attempt, newest first, filterable by cluster, verb,
    /// initiator and outcome. Reads the local audit log only; opening it again shows the open
    /// viewer (with the cluster filter applied).
    #[serde(rename = "audit::Open")]
    AuditOpen {
        /// Show only this cluster's records (the viewer's cluster filter); `None` keeps the
        /// viewer's filters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
    },
    /// Write the open audit viewer's filtered records to a file, as JSONL or CSV (E19-S09). The
    /// user picks the destination; records carry no bodies and are already redacted. Reads only.
    #[serde(rename = "audit::Export")]
    AuditExport {
        /// The file format.
        #[serde(default)]
        format: AuditExportFormat,
    },
    /// Open the Helm charts catalog (E16-S08): search across the configured repositories, chart
    /// cards, and a detail with README, versions and default values. Reads only.
    #[serde(rename = "helm::OpenCharts")]
    HelmOpenCharts,
    /// Search the configured chart repositories (`helm search repo --versions`, E16-S08). Returns
    /// the charts, each with its versions newest first. Reads only.
    #[serde(rename = "helm::SearchCharts")]
    HelmSearchCharts {
        /// What to look for; empty lists every chart.
        #[serde(default)]
        keyword: String,
    },
    /// Print part of a chart (`helm show`, E16-S08): its metadata, README or default values.
    /// Reads only.
    #[serde(rename = "helm::ShowChart")]
    HelmShowChart {
        /// The repository alias.
        repo: String,
        /// The chart name.
        name: String,
        /// The chart version; the newest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// Which part (default: the README).
        #[serde(default)]
        part: crate::helm::ChartPart,
    },
    /// Start installing a chart (E16-S08): hands the chart and version to the install tab
    /// (E16-S09) prefilled. It only opens the tab; nothing is installed until the install itself
    /// goes through the mutation guard.
    #[serde(rename = "helm::InstallChart")]
    HelmInstallChart {
        /// The repository alias.
        repo: String,
        /// The chart name.
        name: String,
        /// The chart version; the newest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// Install a chart as a new release (`helm install`, E16-S09). **Mutating** (risk low): blocked
    /// on a read-only cluster, one confirmation after the preview, audited without the values. A
    /// dry-run dispatch renders the chart (`helm template`) and has the server validate it
    /// (`helm install --dry-run=server`, or `--dry-run` on Helm before 3.13) without installing:
    /// that is the install tab's preview.
    #[serde(rename = "helm::Install")]
    HelmInstall {
        /// The cluster to install into.
        cluster: ClusterId,
        /// The release name (a DNS-1123 label of at most 53 characters).
        release: String,
        /// The namespace.
        namespace: String,
        /// Create the namespace when it is missing (`--create-namespace`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        create_namespace: bool,
        /// The repository alias; absent for a chart reference `helm` resolves itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        /// The chart name.
        chart: String,
        /// The chart version; the newest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// The values overriding the chart's defaults, as YAML. May hold secrets: never logged,
        /// audited or persisted.
        #[serde(default, skip_serializing_if = "crate::helm::ValuesYaml::is_blank")]
        values: crate::helm::ValuesYaml,
        /// Wait for the release's resources to become ready (`--wait`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        wait: bool,
        /// Helm's `--timeout` in seconds; Helm's default (300) when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    /// Open the upgrade tab of a release (E16-S10): the release's current values, a chart version
    /// picker, the values diff, the preview. Reads and changes nothing; the upgrade itself is
    /// `helm::Upgrade`. Without a release the tab lists the cluster's releases to pick from.
    #[serde(rename = "helm::UpgradeRelease")]
    HelmUpgradeRelease {
        /// The release's namespace; absent to pick.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        /// The release's name; absent to pick.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        release: Option<String>,
    },
    /// Upgrade a release to a chart version with values (`helm upgrade`, E16-S10). **Mutating**
    /// (risk medium): blocked on a read-only cluster, one confirmation after the preview, audited
    /// with the chart versions and the `--reuse-values` flag, never the values. A dry-run dispatch
    /// renders the new chart (`helm template`), has the server validate it (`helm upgrade
    /// --dry-run`) and compares its objects with the release's current manifest: the upgrade
    /// tab's preview.
    #[serde(rename = "helm::Upgrade")]
    HelmUpgrade {
        /// The cluster the release is in.
        cluster: ClusterId,
        /// The release's namespace.
        namespace: String,
        /// The release name.
        release: String,
        /// The repository alias; absent for a chart reference `helm` resolves itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        /// The chart name.
        chart: String,
        /// The chart version; the newest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// The values to apply, as YAML. May hold secrets: never logged, audited or persisted.
        #[serde(default, skip_serializing_if = "crate::helm::ValuesYaml::is_blank")]
        values: crate::helm::ValuesYaml,
        /// Keep the previous revision's values under these ones (`--reuse-values`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reuse_values: bool,
        /// Wait for the release's resources to become ready (`--wait`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        wait: bool,
        /// Helm's `--timeout` in seconds; Helm's default (300) when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    /// Roll a release back to an earlier revision (`helm rollback`, E16-S06). **Mutating** (risk
    /// medium): blocked on a read-only cluster, one confirmation, audited. Helm records the
    /// rollback as a new revision, so it can itself be rolled back.
    #[serde(rename = "helm::Rollback")]
    HelmRollback {
        /// The cluster the release is in.
        cluster: ClusterId,
        /// The release's namespace.
        namespace: String,
        /// The release name.
        release: String,
        /// The revision to restore.
        revision: u32,
    },
    /// Remove a release and the objects it made (`helm uninstall`, E16-S06). **Mutating** (risk
    /// high): blocked on a read-only cluster, the user types the release name, audited.
    #[serde(rename = "helm::Uninstall")]
    HelmUninstall {
        /// The cluster the release is in.
        cluster: ClusterId,
        /// The release's namespace.
        namespace: String,
        /// The release name.
        release: String,
        /// Keep the release's history (`--keep-history`), so it can still be rolled back.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        keep_history: bool,
    },
    /// Add a chart repository to Helm's own configuration (`helm repo add`, E16-S07). A password
    /// is never part of a command: the add dialog hands it to the app's repository service in
    /// memory, and an agent cannot supply one. An agent's call needs the user's permission
    /// because it fetches from an arbitrary URL.
    #[serde(rename = "helm::RepoAdd")]
    HelmRepoAdd {
        /// The alias.
        name: String,
        /// The index URL.
        url: String,
        /// Basic-auth user name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        /// Do not verify the server's TLS certificate.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        insecure_skip_tls_verify: bool,
        /// A CA bundle to verify the server with.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ca_file: Option<String>,
    },
    /// Remove a chart repository from Helm's own configuration (`helm repo remove`, E16-S07).
    #[serde(rename = "helm::RepoRemove")]
    HelmRepoRemove {
        /// The alias.
        name: String,
    },
    /// Refresh repository indexes (`helm repo update`, E16-S07): the named ones, or every one
    /// when `names` is empty.
    #[serde(rename = "helm::RepoUpdate")]
    HelmRepoUpdate {
        /// The aliases; empty means all.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
    },
    /// Open the cluster's metrics settings (E13-S05): which Prometheus-compatible service its
    /// history graphs come from, what detection found and which services it probed, the
    /// `prometheus` settings in effect, and the buttons to run detection again and to switch
    /// history metrics on or off. Reads only the cluster's own state.
    #[serde(rename = "cluster::ShowMetricsSettings")]
    ClusterShowMetricsSettings {
        /// The cluster whose metrics settings open.
        cluster: ClusterId,
    },
    /// Open the cluster's pulses (E13-S10): one tile per core kind with its count, how many are
    /// healthy and a short history, plus the cluster's CPU and memory gauges, in a keyboard-driven
    /// page (`:pulses`). Reads only the cluster's own state; opening the page starts the feeds
    /// the tiles count, and closing it releases them.
    #[serde(rename = "cluster::ShowPulses")]
    ClusterShowPulses {
        /// The cluster whose pulses open.
        cluster: ClusterId,
    },
    /// Open the can-i panel (E17-S08, `rbac: can-i`): ask whether the current user, or an
    /// impersonated User, Group or ServiceAccount, may perform a verb on a resource. Read-only:
    /// the answer comes from a `SelfSubjectAccessReview` or `SubjectAccessReview`, which store
    /// nothing. `target` prefills the resource, namespace and name from the selection; the
    /// subject is typed in the panel and never travels in a command or is persisted.
    #[serde(rename = "rbac::CanI")]
    RbacCanI {
        /// The cluster the question is asked of.
        cluster: ClusterId,
        /// The object to prefill the resource, namespace and name from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<ResourceRef>,
        /// The verb to prefill (`get` when absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        verb: Option<String>,
    },
    /// Open the cluster's events view (E14-S03, `:ev`): every event the app's one events feed of
    /// the cluster holds, live, filterable by type, reason, kind, namespace and text, newest
    /// first. Reads only what the `EventService` holds; opening it again shows the open view
    /// (with the given filters applied).
    #[serde(rename = "events::Open")]
    EventsOpen {
        /// The cluster whose events open.
        cluster: ClusterId,
        /// Show only the events stored in this namespace (`:ev kube-system`); an empty name shows
        /// every namespace (`:ev all`). `None` (with no `filter`) keeps the view's filters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        /// The text filter to apply: a case-insensitive substring, or a regular expression when it
        /// has regex syntax (`:ev /BackOff`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<String>,
    },
    /// Show only the `Warning` events in the cluster's events view, or every event again (k9s'
    /// `ctrl-z`). Changes what the view shows, nothing else.
    #[serde(rename = "events::ToggleFaults")]
    EventsToggleFaults {
        /// The cluster whose events view to change.
        cluster: ClusterId,
    },
    /// Pause the cluster's events view's live tail (new events are counted, not shown), or resume
    /// it (they are shown, in order).
    #[serde(rename = "events::TogglePause")]
    EventsTogglePause {
        /// The cluster whose events view to change.
        cluster: ClusterId,
    },
    /// Copy the selected row of the cluster's events view to the clipboard as one line of text
    /// (the message redacted).
    #[serde(rename = "events::CopyRow")]
    EventsCopyRow {
        /// The cluster whose events view to copy from.
        cluster: ClusterId,
    },
    /// Copy the selected event of the cluster's events view to the clipboard as JSON (the message
    /// redacted).
    #[serde(rename = "events::CopyRowJson")]
    EventsCopyRowJson {
        /// The cluster whose events view to copy from.
        cluster: ClusterId,
    },
    /// Write the events the cluster's events view lists (its filters applied, in its order) to a
    /// JSON file the user picks (the messages redacted). Changes nothing in the cluster.
    #[serde(rename = "events::ExportJson")]
    EventsExportJson {
        /// The cluster whose events view to export.
        cluster: ClusterId,
    },
    /// Open the cluster's overview (E13-S09): health tiles (nodes ready, pods by phase), capacity
    /// and allocation bars, top CPU and memory consumers, recent warnings, cluster info, and
    /// history graphs when a Prometheus-compatible endpoint answers. Reads only what the session's
    /// store and the metrics services already hold.
    #[serde(rename = "cluster::ShowOverview")]
    ClusterShowOverview {
        /// The cluster whose overview opens.
        cluster: ClusterId,
    },
    /// Run the metrics provider detection for a cluster again (E13-S05): list the Services of the
    /// namespaces the monitoring stacks use, probe the ones that match with a cheap query through
    /// the API server's service proxy, and classify the stack. A cluster configured by hand has
    /// its endpoint checked instead. Read-only: it lists Services and sends one query.
    #[serde(rename = "cluster::RedetectMetrics")]
    ClusterRedetectMetrics {
        /// The cluster to look in.
        cluster: ClusterId,
    },
    /// Switch a cluster's history metrics on or off (E13-S05): `prometheus.enabled` in its
    /// settings. `Some` sets it, `None` flips the current value.
    ///
    /// Not a cluster mutation: it only edits the cluster's own settings, so it runs on a
    /// read-only cluster. Live CPU and memory from metrics-server are not affected.
    #[serde(rename = "cluster::ToggleMetrics")]
    ClusterToggleMetrics {
        /// The cluster to change.
        cluster: ClusterId,
        /// Desired state; `None` flips the current one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
    },
    /// Queue Kubernetes events as context for the hosted agent (E14-S08), with where they came
    /// from: the events named in `events` (the events view's selection, by uid), else every held
    /// event about `target` (a resource's Events tab, a table row), else the cluster's newest
    /// Warning events. The agent panel (E27) drains the queue; until then it waits locally.
    /// Reads the app's event buffer and changes nothing in the cluster.
    #[serde(rename = "events::SendToAgent")]
    EventsSendToAgent {
        /// The cluster whose events to send.
        cluster: ClusterId,
        /// The object whose events to send.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<ResourceRef>,
        /// The uids of the events to send; empty: use `target`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<String>,
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
    /// Select all namespaces (`slot` 0) or the nth favourite (`slot` 1 to 9) of a cluster, as
    /// k9s's number keys do. A slot with no favourite changes nothing.
    #[serde(rename = "namespace::UseFavourite")]
    NamespaceUseFavourite {
        /// The cluster whose selection changes.
        cluster: ClusterId,
        /// `0` for all namespaces, `1` to `9` for the nth favourite.
        slot: u8,
    },
    /// Create a namespace (E12-S16): a manifest of the name and labels, created under the guard.
    #[serde(rename = "namespace::Create")]
    NamespaceCreate {
        /// The cluster to create it in.
        cluster: ClusterId,
        /// The namespace name (a DNS label).
        name: String,
        /// Labels to set on it.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        labels: BTreeMap<String, String>,
    },
    /// Delete a namespace and everything in it (E12-S16). Irreversible: the user types the
    /// name. Deleting is asynchronous: the namespace goes `Terminating` until its contents and
    /// finalizers are gone.
    #[serde(rename = "namespace::Delete")]
    NamespaceDelete {
        /// The namespace.
        target: ResourceRef,
        /// How many objects of each kind the namespace held when the user asked (kind to count),
        /// kept in the audit record. Informational: empty when the caller did not look.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        contents: BTreeMap<String, u32>,
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
    /// Discover the managed clusters of a cloud provider with its CLI (`aws`, `gcloud`, `az`), or
    /// of every provider. Reads from the provider and caches the listing; changes nothing.
    #[serde(rename = "cloud::Refresh")]
    CloudRefresh {
        /// The provider; `None` refreshes all of them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<CloudProvider>,
        /// Whether a fresh cache may answer.
        #[serde(default)]
        mode: CloudRefreshMode,
    },
    /// Stop a running cloud discovery (its CLI processes are killed).
    #[serde(rename = "cloud::Cancel")]
    CloudCancel {
        /// The provider; `None` stops all of them.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<CloudProvider>,
    },
    /// Add a discovered cloud cluster to the catalog: an exec-auth entry in Oxikube's own managed
    /// kubeconfig for the provider (no token is written; the user's kubeconfig is not touched).
    #[serde(rename = "cloud::Add")]
    CloudAdd {
        /// The provider.
        provider: CloudProvider,
        /// The discovered cluster's key (`aws/prod/eu-west-1/prod-eu`), from the last listing.
        cluster: String,
    },
    /// Remove a cloud cluster that was added: its managed kubeconfig entry goes and an open
    /// session of it is disconnected.
    #[serde(rename = "cloud::Remove")]
    CloudRemove {
        /// The provider.
        provider: CloudProvider,
        /// The context name of the added cluster.
        context: ContextName,
    },
    /// Open a registered view (overview, events, ...) by id.
    #[serde(rename = "view::Open")]
    ViewOpen {
        /// View id as registered by the owning ui crate.
        view: String,
    },
    /// Show or hide the command palette.
    #[serde(rename = "palette::Toggle")]
    PaletteToggle,
    /// Forget the commands the palette lists first because they ran lately.
    #[serde(rename = "palette::ClearRecents")]
    PaletteClearRecents,
    /// Open the `:` jump bar (`:` in a cluster tab).
    #[serde(rename = "palette::OpenJump")]
    PaletteOpenJump,
    /// Show the key bindings that apply where the focus is (`?`).
    #[serde(rename = "help::Show")]
    HelpShow,
    /// List the commands that cannot run in the open command palette too (marked, with the
    /// reason), or hide them again.
    #[serde(rename = "palette::ToggleShowAll")]
    PaletteToggleShowAll,
    /// Open the user's `keymap.json` in an editor, creating it from a commented template first
    /// when it does not exist.
    #[serde(rename = "keymap::OpenUser")]
    KeymapOpenUser,
    /// Open the chart gallery, every chart state on one page, as a tab.
    #[serde(rename = "charts::OpenGallery")]
    ChartsOpenGallery,
    /// Quit the application; asks first while operations are running.
    #[serde(rename = "app::Quit")]
    AppQuit,
    /// Show one OS (desktop) notification, to check that they reach the desktop and that a click
    /// brings Oxikube to the front (E14-S06). Shows nothing while OS notifications are off.
    #[serde(rename = "app::SendTestNotification")]
    AppSendTestNotification,
    /// Turn OS (desktop) notifications on or off: writes `events.os_notifications` in the user's
    /// settings. Turning them on is the opt-in, and the only moment the OS is asked for
    /// permission. Not a cluster mutation, so it runs on a read-only cluster.
    #[serde(rename = "app::ToggleOsNotifications")]
    AppToggleOsNotifications {
        /// Desired state; `None` flips the current one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled: Option<bool>,
    },
    /// Detect the `helm` binary again (after installing Helm or changing `helm.path`) and refresh
    /// what the Helm views offer (E16-S11). Runs `helm version`; changes nothing in any cluster.
    #[serde(rename = "helm::Recheck")]
    HelmRecheck,
    /// Open the `helm.path` setting so the user can point Oxikube at their `helm` binary
    /// (E16-S11). Changes nothing in any cluster.
    #[serde(rename = "helm::LocateBinary")]
    HelmLocateBinary,
    /// Show and focus the notification centre, or hide it (E14-S05).
    #[serde(rename = "notifications::TogglePanel")]
    NotificationsTogglePanel,
    /// Mark one notification of the notification centre read (E14-S05).
    #[serde(rename = "notifications::MarkRead")]
    NotificationsMarkRead {
        /// The notification's id, as the centre lists it.
        id: u64,
    },
    /// Mark every notification of the notification centre read (E14-S05).
    #[serde(rename = "notifications::MarkAllRead")]
    NotificationsMarkAllRead,
    /// Open the `https` link of a notification (a release page) in the browser (E14-S07).
    /// Anything but an `https` URL is refused.
    #[serde(rename = "notifications::OpenLink")]
    NotificationsOpenLink {
        /// The link, as the notification carries it.
        url: String,
    },
    /// Stop notifying about a cluster's events until it is unmuted; remembered across restarts
    /// (E14-S05). A local preference, not a cluster mutation.
    #[serde(rename = "notifications::MuteCluster")]
    NotificationsMuteCluster {
        /// The cluster to mute.
        cluster: ClusterId,
    },
    /// Notify about a muted cluster's events again (E14-S05).
    #[serde(rename = "notifications::UnmuteCluster")]
    NotificationsUnmuteCluster {
        /// The cluster to unmute.
        cluster: ClusterId,
    },
    /// Run the previous command of the jump bar's history again (`[`).
    #[serde(rename = "jump::Back")]
    JumpBack,
    /// Run the next command of the jump bar's history again (`]`).
    #[serde(rename = "jump::Forward")]
    JumpForward,
    /// Go back to the view before the current one (`-`), and back again on a second use.
    #[serde(rename = "jump::Last")]
    JumpLast,
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
    /// Clear the focused terminal (E09-S11): its scrollback is dropped and the lines above the cursor are cleared, so the prompt moves to the top. The process is not told and is not interrupted. Changes nothing in a cluster.
    #[serde(rename = "terminal::Clear")]
    TerminalClear,
    /// Scroll the focused terminal's history one line towards the live screen (E09-S11).
    #[serde(rename = "terminal::ScrollLineDown")]
    TerminalScrollLineDown,
    /// Scroll the focused terminal's history one line up (E09-S11).
    #[serde(rename = "terminal::ScrollLineUp")]
    TerminalScrollLineUp,
    /// Scroll the focused terminal's history one screen towards the live screen (E09-S11).
    #[serde(rename = "terminal::ScrollPageDown")]
    TerminalScrollPageDown,
    /// Scroll the focused terminal's history one screen up (E09-S11).
    #[serde(rename = "terminal::ScrollPageUp")]
    TerminalScrollPageUp,
    /// Open the search bar of the focused terminal (E09-S11) to find text, as a regular expression, in its screen and scrollback.
    #[serde(rename = "terminal::Search")]
    TerminalSearch,
    /// Close the focused terminal's search bar and hand the keyboard back to the process (E09-S11).
    #[serde(rename = "terminal::SearchClose")]
    TerminalSearchClose,
    /// Jump to the next match of the focused terminal's search, wrapping at the end (E09-S11).
    #[serde(rename = "terminal::SearchNext")]
    TerminalSearchNext,
    /// Jump to the previous match of the focused terminal's search, wrapping at the start (E09-S11).
    #[serde(rename = "terminal::SearchPrevious")]
    TerminalSearchPrevious,
    /// Select the whole screen and scrollback of the focused terminal (E09-S11), ready to copy.
    #[serde(rename = "terminal::SelectAll")]
    TerminalSelectAll,
    /// Open a link a terminal shows (cmd/ctrl-click, E09-S05): an `http`, `https`, `mailto` or
    /// `file` URL in the browser, or an absolute local path (an optional `:line[:column]` suffix
    /// is accepted) with the system's opener. Reads and changes nothing in a cluster.
    #[serde(rename = "terminal::OpenLink")]
    TerminalOpenLink {
        /// The URL or absolute path.
        target: String,
    },
    /// Replace the focused manifest editor's YAML with the same data as JSON (E10-S10), as one
    /// undo step; comments cannot be carried over and are counted in a warning. The active
    /// pane's editor when focus is elsewhere. Edits the buffer only.
    #[serde(rename = "editor::ConvertToJson")]
    EditorConvertToJson,
    /// Replace the focused manifest editor's JSON with the same data as YAML (E10-S10), as one
    /// undo step. The active pane's editor when focus is elsewhere. Edits the buffer only.
    #[serde(rename = "editor::ConvertToYaml")]
    EditorConvertToYaml,
    /// Read the focused manifest editor's buffer as JSON (E10-S10): the JSON grammar and the
    /// strict JSON checks, text untouched. Overrides the detection.
    #[serde(rename = "editor::UseJson")]
    EditorUseJson,
    /// Read the focused manifest editor's buffer as YAML (E10-S10); the text is untouched.
    /// Overrides the detection.
    #[serde(rename = "editor::UseYaml")]
    EditorUseYaml,
    /// Open an empty manifest editor (E10-S04): a tab of `cluster`'s tab (the displayed cluster
    /// tab when `None`), whose YAML is checked against that cluster's schemas, or a tab of the
    /// window with syntax checks only when no cluster tab is shown. Edits text only; applying it
    /// is `resource::Apply`.
    #[serde(rename = "editor::NewManifest")]
    EditorNewManifest {
        /// The cluster whose tab gets the editor; the displayed cluster tab when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
    },
    /// Show or hide `metadata.managedFields` in the focused manifest editor of a resource
    /// (E10-S06); the active pane's editor when focus is elsewhere. The text is rendered again
    /// from the object the editor holds.
    #[serde(rename = "editor::ToggleManagedFields")]
    EditorToggleManagedFields,
    /// Show the values of a Secret in the focused manifest editor of a resource, or mask them
    /// again (E10-S06). The values stay in memory; the revealed text is never written.
    #[serde(rename = "editor::ToggleSecretMask")]
    EditorToggleSecretMask,
    /// Make the focused manifest editor read-only, or editable again (E10-S04); the active
    /// pane's editor when focus is elsewhere. A view state, not the cluster's read-only mode.
    #[serde(rename = "editor::ToggleReadOnly")]
    EditorToggleReadOnly,
    /// Wrap the focused manifest editor's long lines at its width, or let them scroll sideways
    /// (E10-S04); the active pane's editor when focus is elsewhere.
    #[serde(rename = "editor::ToggleSoftWrap")]
    EditorToggleSoftWrap,
    /// Show the focused manifest editor's diff against the live object in its bottom panel, or
    /// hide it (E10-S07): the object the buffer was loaded from, else the one its manifest
    /// names. Comments, key order and server-owned fields are ignored. Reads the cluster only.
    #[serde(rename = "editor::ShowDiff")]
    EditorShowDiff,
    /// Switch the focused editor's diff between side by side and inline (E10-S07).
    #[serde(rename = "editor::ToggleDiffLayout")]
    EditorToggleDiffLayout,
    /// Replace the focused editor's buffer with the live object, dropping its edits (E10-S07,
    /// the changed-on-server banner's "Reload"). A buffer not loaded from an object loads the
    /// one its manifest names. Reads the cluster only.
    #[serde(rename = "editor::ReloadFromServer")]
    EditorReloadFromServer,
    /// Merge the server's change into the focused editor's buffer, three-way against the
    /// version it was loaded from (E10-S07, the banner's "Merge"). Conflicts wait for a side.
    #[serde(rename = "editor::MergeFromServer")]
    EditorMergeFromServer,
    /// Keep the focused editor's buffer as it is over the server's change (E10-S07, the
    /// banner's "Keep mine"); a merge waiting for sides is dropped.
    #[serde(rename = "editor::KeepMine")]
    EditorKeepMine,
    /// Settle conflict `conflict` (counting from 0) of the focused editor's merge with the
    /// buffer's lines (E10-S07).
    #[serde(rename = "editor::TakeMine")]
    EditorTakeMine {
        /// The conflict, counting from 0.
        conflict: u32,
    },
    /// Settle conflict `conflict` (counting from 0) of the focused editor's merge with the
    /// server's lines (E10-S07).
    #[serde(rename = "editor::TakeServer")]
    EditorTakeServer {
        /// The conflict, counting from 0.
        conflict: u32,
    },
    /// Put the focused editor's merge in its buffer once every conflict has a side (E10-S07).
    #[serde(rename = "editor::AcceptMerge")]
    EditorAcceptMerge,
    /// Open the completion menu at the focused manifest editor's cursor (E10-S05): the schema's
    /// sibling fields, enum values and snippets; the active pane's editor when focus is
    /// elsewhere. Reads the text only.
    #[serde(rename = "editor::TriggerCompletion")]
    EditorTriggerCompletion,
    /// Show the schema docs of the token at the focused manifest editor's cursor (E10-S05), as
    /// hovering it would; the active pane's editor when focus is elsewhere. Reads the text only.
    #[serde(rename = "editor::ShowHover")]
    EditorShowHover,
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
    /// Open the find field of the resource detail's YAML or Describe text, optionally with
    /// `pattern` already typed (`/` in the detail). The search is a view of the text on screen:
    /// it reads nothing from the cluster.
    #[serde(rename = "resource::Find")]
    ResourceFind {
        /// The resource whose detail is searched.
        target: ResourceRef,
        /// The text or regular expression to search for; `None` just opens the field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    /// Go to the next match of the detail's find, wrapping from the last to the first (`n`).
    #[serde(rename = "resource::NextMatch")]
    ResourceNextMatch {
        /// The resource whose detail is searched.
        target: ResourceRef,
    },
    /// Go to the previous match of the detail's find, wrapping from the first to the last (`N`).
    #[serde(rename = "resource::PreviousMatch")]
    ResourcePreviousMatch {
        /// The resource whose detail is searched.
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
    /// Forward a local port to a pod behind a Service and open `http://localhost:<port>` in the
    /// browser. Offered only where a port-forward manager is installed.
    #[serde(rename = "service::OpenInBrowser")]
    ServiceOpenInBrowser {
        /// The Service.
        target: ResourceRef,
        /// The Service port to open; `None` opens the first TCP port.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
    /// Open the table of the pods a Service selects, filtered by its selector (`-l app=web`).
    #[serde(rename = "service::ShowPods")]
    ServiceShowPods {
        /// The Service.
        target: ResourceRef,
    },
    /// Show what uses a ServiceAccount, ConfigMap, Secret, PersistentVolumeClaim or PriorityClass:
    /// its detail opens on the "Used by" tab (E12-S18; `u`, k9s's used-by). The workloads come from
    /// the Pods' specs; nothing is changed or written.
    #[serde(rename = "resource::ShowUsedBy")]
    ResourceShowUsedBy {
        /// The object to show the users of.
        target: ResourceRef,
    },
    /// Compare a resource with the same kind in another cluster or namespace (E20-S09): the two
    /// objects are read side by side, normalised (server-owned fields, the `diff.ignore_paths`
    /// setting) and shown in a diff tab with a changed-path summary above it. **Read-only**: no
    /// guard tier, nothing is written. A Secret's values never appear, only `changed` or `same`
    /// per key. Without `other`, the "Diff with..." picker asks for the second object.
    #[serde(rename = "resource::DiffWith")]
    ResourceDiffWith {
        /// The object to compare (the left side).
        target: ResourceRef,
        /// The object to compare it with (the right side); the picker opens when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other: Option<ResourceRef>,
        /// Compare `status` too (default: ignored, it always differs).
        #[serde(default)]
        include_status: bool,
        /// Compare `metadata.name` and `metadata.namespace` too (default: ignored, they differ by
        /// design when comparing across namespaces).
        #[serde(default)]
        compare_identity: bool,
    },
    /// Request a token for a ServiceAccount and build a kubeconfig from it (E17-S09). **Mutating**
    /// (risk low): it creates a credential, so it is blocked on a read-only cluster, confirmed once
    /// and audited with the account and the lifetime, never the token. The result stays in memory
    /// (`oxikube_app::sa_kubeconfig::KubeconfigVault`); the output carries only a message and the
    /// expiry.
    #[serde(rename = "serviceaccount::GenerateKubeconfig")]
    ServiceAccountGenerateKubeconfig {
        /// The ServiceAccount.
        target: ResourceRef,
        /// The token's lifetime in seconds (the server may shorten it); one hour when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expiration_seconds: Option<u32>,
        /// The token's audiences; the API server's own when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        audiences: Vec<String>,
        /// On a cluster without `TokenRequest`, allow reading or creating a long-lived token
        /// Secret. Without it such a cluster is refused with an explanation.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_legacy_token: bool,
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
    /// Put `text` in the filter bar of a kind's resource table and apply it, as if typed:
    /// `/text`, `/!text`, `/-l selector`, `/-f fuzzy`, or `text -l selector` for a name filter and
    /// a selector together. An empty `text` clears the filter. When the table is not open yet,
    /// the filter is waiting for it. This is what the jump bar's `:pod /api app=x` ends in.
    #[serde(rename = "table::SetFilter")]
    TableSetFilter {
        /// Cluster of the list.
        cluster: ClusterId,
        /// The kind listed.
        gvk: Gvk,
        /// The filter, in the filter bar's grammar.
        text: String,
    },
    /// Open a resource's YAML.
    #[serde(rename = "resource::ViewYaml")]
    ResourceViewYaml {
        /// The resource to show.
        target: ResourceRef,
    },
    /// Open a resource's YAML in a read-only manifest editor tab titled `kind/name` (E10-S06):
    /// Secret values masked, `managedFields` hidden. The tab of the same object is reused.
    #[serde(rename = "resource::ViewYamlInEditor")]
    ResourceViewYamlInEditor {
        /// The resource to show.
        target: ResourceRef,
    },
    /// Open a resource's `kubectl describe` text (`d` in a resource table).
    #[serde(rename = "resource::ViewDescribe")]
    ResourceViewDescribe {
        /// The resource to describe.
        target: ResourceRef,
    },
    /// Open a resource's detail on its Events tab: the events about the object from the app's
    /// events index, newest first (E14-S04). The warning badge of a table row sends it.
    #[serde(rename = "resource::ViewEvents")]
    ResourceViewEvents {
        /// The resource whose events to show.
        target: ResourceRef,
    },
    /// Open a resource's detail on its Metrics tab: the graphs of the last 15 minutes to 24 hours
    /// from the cluster's Prometheus (E13-S08). Pods, nodes, workloads, namespaces, PVCs and
    /// ingresses have it; without a Prometheus the command says why instead.
    #[serde(rename = "resource::ViewMetrics")]
    ResourceViewMetrics {
        /// The resource to show graphs for.
        target: ResourceRef,
    },
    /// Open a resource's manifest in the editor (`e` in a resource table). The edit reaches the
    /// cluster only through `resource::Apply`.
    #[serde(rename = "resource::Edit")]
    ResourceEdit {
        /// The resource to edit.
        target: ResourceRef,
    },
    /// Show or hide the wide columns (`kubectl -o wide`) of a kind's resource tables (`ctrl-w`
    /// in a table, k9s's wide toggle).
    #[serde(rename = "table::ToggleWide")]
    TableToggleWide {
        /// Cluster of the list.
        cluster: ClusterId,
        /// The kind listed.
        gvk: Gvk,
    },
    /// Open the "Create resource" picker in the manifest editor (E10-S09): the embedded templates
    /// of the core kinds and the user's own, with the namespace of the cluster's selection filled
    /// in. With `template` the editor opens on that template at once. Edits text only; the
    /// objects are made by `resource::Apply`, one per document.
    #[serde(rename = "resource::Create")]
    ResourceCreate {
        /// The cluster to create in; the displayed cluster tab when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
        /// The template to open (by name, `Deployment`); the picker when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    /// Show a Secret's values decoded in its detail, or hide them again (E12-S11; `x` on a
    /// Secret, k9s's decode). The values are read once and held in memory for as long as the
    /// detail shows them; nothing is written or logged. With `key`, only that key.
    #[serde(rename = "secret::Reveal")]
    SecretReveal {
        /// The Secret.
        target: ResourceRef,
        /// The one key to reveal or hide; every key when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// Open a Secret in the manifest editor with its values decoded to plaintext (E12-S11). The
    /// buffer is re-encoded to base64 when it is applied (`resource::Apply`); the decoded text is
    /// never saved with the layout.
    #[serde(rename = "secret::EditDecoded")]
    SecretEditDecoded {
        /// The Secret to edit.
        target: ResourceRef,
    },
    /// Open the "Create secret" dialog (E12-S11): Opaque key/value rows, a Docker registry login
    /// or a TLS pair, validated before an editor tab with the manifest opens. The object is made
    /// by `resource::Apply`.
    #[serde(rename = "secret::Create")]
    SecretCreate {
        /// The cluster to create in; the displayed cluster tab when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
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
    /// Apply a manifest of one object (E10-S08): server-side apply under a field manager, after a
    /// server dry run; a replace carrying `resource_version` where the server cannot apply. A
    /// dry-run dispatch stops after the dry run and returns the object as the server would store
    /// it, next to the live one.
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
        /// The server-side apply field manager; `None` is `oxikube`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field_manager: Option<String>,
        /// Take over the fields other field managers own instead of failing with a conflict.
        /// Off unless the user chose it after seeing the conflicts.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
        /// The live object's `metadata.resourceVersion` the change was reviewed against. The
        /// replace fallback carries it, so a change made since fails with a conflict instead of
        /// being overwritten.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_version: Option<String>,
        /// The change was reviewed against no live object (the preview showed a create). The
        /// fallback then creates, so an object made since fails with `AlreadyExists` instead of
        /// being replaced unseen.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reviewed_absent: bool,
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
    /// Evict one pod through the eviction subresource, which honours PodDisruptionBudgets (E12-S03).
    /// A budget that forbids it refuses with "blocked by disruption budget".
    #[serde(rename = "pod::Evict")]
    PodEvict {
        /// The pod to evict.
        target: ResourceRef,
    },
    /// Delete one pod with grace period 0 (k9s's kill, `ctrl-k`; `kubectl delete --force
    /// --grace-period=0`, E12-S03): the kubelet is not waited for, so a container may still run
    /// on a node that cannot be reached. Confirmed by typing the pod's name.
    #[serde(rename = "pod::ForceDelete")]
    PodForceDelete {
        /// The pod to delete.
        target: ResourceRef,
    },
    /// Delete the Completed (`Succeeded`) and `Failed` pods of a view (k9s's `z`, E12-S03). With
    /// `targets` the pods the view shows; without, every such pod of `namespace` (or of the whole
    /// cluster). Each pod is re-read and skipped unless it still is Completed or Failed, and is
    /// deleted and audited on its own under one shared batch id.
    #[serde(rename = "pod::Sanitize")]
    PodSanitize {
        /// The cluster.
        cluster: ClusterId,
        /// The namespace to sweep when `targets` is empty; `None`: every namespace.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        /// The pods to sweep (the table's rows in view); empty: list them from the cluster.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<ResourceRef>,
    },
    /// Open the controller of a pod (its ReplicaSet, a Job's pod's Job, a static pod's Node): k9s's
    /// `shift-j` (E12-S03). Reads the pod; changes nothing.
    #[serde(rename = "pod::JumpToOwner")]
    PodJumpToOwner {
        /// The pod.
        target: ResourceRef,
    },
    /// Open the Node a pod runs on: k9s's `o` (E12-S03). Reads the pod; changes nothing.
    #[serde(rename = "pod::ShowNode")]
    PodShowNode {
        /// The pod.
        target: ResourceRef,
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
    /// Add an ephemeral debug container to a running pod (`kubectl debug`) and open a terminal
    /// in it once it runs. A mutation (low risk, simple confirm): it patches the pod's
    /// `ephemeralcontainers` subresource, and an ephemeral container can be neither removed nor
    /// edited afterwards, so the confirmation says so.
    #[serde(rename = "pod::Debug")]
    PodDebug {
        /// The pod to debug.
        target: ResourceRef,
        /// The image to run ([`DEFAULT_DEBUG_IMAGE`] when left out).
        #[serde(default = "default_debug_image")]
        image: String,
        /// The container whose process namespace the debug container shares; `None` picks the
        /// pod's default container.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_container: Option<String>,
        /// The program and arguments to run; empty runs [`DEFAULT_DEBUG_COMMAND`].
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        command: Vec<String>,
        /// The new container's name; `None` generates `debugger-xxxxx`. A name cannot be reused
        /// in the pod.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
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
    /// Start a port-forward to a pod or a Service (E15-S03). A Service resolves to one of its
    /// ready pods, and the forward follows the pod when it is replaced. Not a cluster mutation;
    /// refused on a read-only cluster unless `portforward.allowed_in_readonly`.
    #[serde(rename = "portforward::Start")]
    PortForwardStart {
        /// The core `v1` `Pod` or `Service` to forward to.
        target: ResourceRef,
        /// The port on the target (for a Service, the Service port).
        remote_port: ForwardPort,
        /// Local port; `None` picks a free one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_port: Option<u16>,
        /// Local address to listen on; `None` is loopback.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind: Option<IpAddr>,
        /// The name the user gave the forward; `None` is `<kind>/<name>:<port>`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Whether to also save the forward as an auto-start favourite once it is accepted (the
        /// start dialog's "Keep as a favourite", E15-S06).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        favourite: bool,
    },
    /// Stop a port-forward; it stays listed and can be restarted (E15-S03).
    #[serde(rename = "portforward::Stop")]
    PortForwardStop {
        /// The forward's id, as the manager lists it.
        id: u64,
    },
    /// Restart a port-forward: stopped, failed or running, it starts again (E15-S03).
    #[serde(rename = "portforward::Restart")]
    PortForwardRestart {
        /// The forward's id, as the manager lists it.
        id: u64,
    },
    /// Stop a port-forward and remove it from the list (E15-S03).
    #[serde(rename = "portforward::Remove")]
    PortForwardRemove {
        /// The forward's id, as the manager lists it.
        id: u64,
    },
    /// Save a port-forward as a favourite (E15-S06): restored when the cluster connects unless
    /// `auto_start` is off. Saving the same forward again updates its name and switch. Holds no
    /// secret: a cluster id, a resource reference, ports, a bind address and a display name.
    #[serde(rename = "portforward::FavouriteAdd")]
    PortForwardFavouriteAdd {
        /// The core `v1` `Pod` or `Service` the favourite forwards to.
        target: ResourceRef,
        /// The port on the target (for a Service, the Service port).
        remote_port: ForwardPort,
        /// Local port; `None` is "Auto" (a free port each time).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_port: Option<u16>,
        /// Local address to listen on; `None` is loopback.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bind: Option<IpAddr>,
        /// A display name; `None` names it after the target and port.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Whether to start it when the cluster connects; `None` is on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auto_start: Option<bool>,
    },
    /// Remove a favourite; a forward running from it keeps running (E15-S06).
    #[serde(rename = "portforward::FavouriteRemove")]
    PortForwardFavouriteRemove {
        /// The favourite's id, as the favourites list it.
        id: String,
    },
    /// Switch a favourite's start-on-connect on or off (E15-S06).
    #[serde(rename = "portforward::FavouriteToggleAutoStart")]
    PortForwardFavouriteToggleAutoStart {
        /// The favourite's id, as the favourites list it.
        id: String,
    },
    /// Copy a port-forward's local URL to the clipboard (E15-S05).
    #[serde(rename = "portforward::CopyUrl")]
    PortForwardCopyUrl {
        /// The forward's id, as the manager lists it.
        id: u64,
    },
    /// Open a port-forward's local URL in the system browser (E15-S05).
    #[serde(rename = "portforward::OpenInBrowser")]
    PortForwardOpenInBrowser {
        /// The forward's id, as the manager lists it.
        id: u64,
    },
    /// Show the Port Forwards panel and focus it, or hide it (E15-S05). `show` only ever opens it
    /// (`:pf`, the table's `f`); without it a shown panel is hidden again.
    #[serde(rename = "portforward::TogglePanel")]
    PortForwardTogglePanel {
        /// Open the panel instead of toggling it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        show: bool,
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
    /// Rolling-restart a workload: stamp its pod template with a new
    /// `kubectl.kubernetes.io/restartedAt` so its controller rolls fresh pods
    /// (`kubectl rollout restart`, E12-S05).
    #[serde(rename = "workload::Restart")]
    WorkloadRestart {
        /// The Deployment, StatefulSet or DaemonSet to restart.
        target: ResourceRef,
    },
    /// Change the image of one or more containers of a workload's pod template
    /// (`kubectl set image`, E12-S05), which rolls the workload.
    #[serde(rename = "workload::SetImage")]
    WorkloadSetImage {
        /// The Deployment, StatefulSet or DaemonSet.
        target: ResourceRef,
        /// The containers (or init containers) to change and their new images.
        images: Vec<ImageChange>,
    },
    /// Restore a workload's pod template from an earlier revision of its rollout history, as
    /// `kubectl rollout undo` does (E12-S06): a Deployment from the ReplicaSet of that revision, a
    /// StatefulSet or DaemonSet from its ControllerRevision. Given a ReplicaSet, the Deployment
    /// that owns it is rolled back to that ReplicaSet's revision.
    #[serde(rename = "workload::Rollback")]
    WorkloadRollback {
        /// The Deployment, StatefulSet, DaemonSet or ReplicaSet (of a Deployment).
        target: ResourceRef,
        /// The revision to restore; `None` is the previous one (the second newest), as kubectl's
        /// default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_revision: Option<i64>,
    },
    /// Delete a selection of objects of any kinds, as one batch (E12-S19): one confirmation sized
    /// to the riskiest object, then each object is deleted and audited on its own under a shared
    /// batch id. The result lists every object (`data.rows`).
    #[serde(rename = "selection::Delete")]
    SelectionDelete {
        /// The objects, all of one cluster.
        targets: Vec<ResourceRef>,
        /// How dependents are handled; the kubectl default (background) unless chosen.
        #[serde(default)]
        propagation: Propagation,
    },
    /// Rolling-restart a selection of Deployments, StatefulSets and DaemonSets, as one batch
    /// (E12-S19). Anything else in the selection is refused up front.
    #[serde(rename = "selection::Restart")]
    SelectionRestart {
        /// The workloads, all of one cluster.
        targets: Vec<ResourceRef>,
    },
    /// Add, change or remove labels (or annotations) on a selection of objects of any kinds, as
    /// one batch (E12-S19): the same merge patch for every object, each audited on its own.
    #[serde(rename = "selection::Label")]
    SelectionLabel {
        /// The objects, all of one cluster.
        targets: Vec<ResourceRef>,
        /// Whether the edits are to labels or annotations.
        #[serde(default)]
        field: LabelField,
        /// The keys to set or remove.
        edits: Vec<LabelEdit>,
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
    /// Make an IngressClass the cluster's default by annotating it
    /// `ingressclass.kubernetes.io/is-default-class: "true"`.
    #[serde(rename = "ingressclass::SetDefault")]
    IngressClassSetDefault {
        /// The IngressClass to make the default.
        target: ResourceRef,
        /// Also clear the annotation on the classes that carried it, so exactly one default
        /// remains (the API server refuses an Ingress without a class while two exist).
        /// Defaults to `true`.
        #[serde(default = "default_follow")]
        clear_previous: bool,
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
    /// Open a shell on a node through a privileged helper pod (`kubectl debug node` style): the
    /// pod is created on the node from the node shell settings, the shell runs in the node's
    /// namespaces, and the pod is deleted when the terminal closes. Creating a privileged pod
    /// is a mutation: blocked on a read-only cluster, confirmed with the node and the image
    /// named, audited.
    #[serde(rename = "node::Shell")]
    NodeShell {
        /// The node.
        target: ResourceRef,
    },
    /// Cordon a node and evict its pods the way `kubectl drain` does (E12-S10): DaemonSet and
    /// mirror pods skipped, PodDisruptionBudgets waited out, per-pod progress.
    #[serde(rename = "node::Drain")]
    NodeDrain {
        /// The node.
        target: ResourceRef,
        /// The `kubectl drain` flags (`ignore_daemonsets`, `delete_emptydir_data`, `force`,
        /// `grace_period_secs`, `timeout_secs`), flat in the payload; missing ones take the
        /// [`DrainFlags`](crate::drain::DrainFlags) defaults.
        #[serde(flatten)]
        flags: crate::drain::DrainFlags,
    },
    /// Run a CronJob now: create a Job from its `spec.jobTemplate` with a generated name and the
    /// CronJob as its owner (`kubectl create job --from=cronjob/x`, E12-S08).
    #[serde(rename = "cronjob::Trigger")]
    CronJobTrigger {
        /// The CronJob.
        target: ResourceRef,
    },
    /// Stop a CronJob creating Jobs (`spec.suspend: true`).
    #[serde(rename = "cronjob::Suspend")]
    CronJobSuspend {
        /// The CronJob.
        target: ResourceRef,
    },
    /// Let a suspended CronJob create Jobs again (`spec.suspend: false`).
    #[serde(rename = "cronjob::Resume")]
    CronJobResume {
        /// The CronJob.
        target: ResourceRef,
    },
    /// Suspend a Job: its controller deletes the running pods until it is resumed
    /// (`spec.suspend: true`).
    #[serde(rename = "job::Suspend")]
    JobSuspend {
        /// The Job.
        target: ResourceRef,
    },
    /// Resume a suspended Job (`spec.suspend: false`); a Job that finished cannot be resumed.
    #[serde(rename = "job::Resume")]
    JobResume {
        /// The Job.
        target: ResourceRef,
    },
    /// Grow a PersistentVolumeClaim: patch `spec.resources.requests.storage`. Refused when the
    /// new size is not greater than the current one, or when the claim's StorageClass does not
    /// allow volume expansion (`oxikube_domain::view::storage::check_resize`).
    #[serde(rename = "pvc::Resize")]
    PvcResize {
        /// The PersistentVolumeClaim.
        target: ResourceRef,
        /// The new size as a Kubernetes quantity (`20Gi`).
        size: String,
        /// The size the caller saw (`10Gi`). Recorded in the audit entry next to the new size,
        /// and checked against the live claim: if the claim changed meanwhile the resize is
        /// refused instead of applied on a size the user did not see.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    /// Make a StorageClass the cluster's default: set the
    /// `storageclass.kubernetes.io/is-default-class` annotation to `true`.
    #[serde(rename = "storageclass::SetDefault")]
    StorageClassSetDefault {
        /// The StorageClass.
        target: ResourceRef,
        /// Also clear the default annotation on the class(es) that were the default, so the
        /// cluster is left with exactly one (default `true`).
        #[serde(default = "default_clear_previous")]
        clear_previous: bool,
    },
    /// Open the Role / ClusterRole rule dialog (E17-S05): a new Role, a new ClusterRole, or the
    /// rules of the existing `target`. Changes nothing; the dialog's save is [`RoleCreate`] or
    /// [`RoleEdit`].
    ///
    /// [`RoleCreate`]: Command::RoleCreate
    /// [`RoleEdit`]: Command::RoleEdit
    #[serde(rename = "role::OpenDialog")]
    RoleOpenDialog {
        /// The cluster whose tab gets the dialog; the shown one when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
        /// A new ClusterRole instead of a new Role (ignored when `target` is set).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        cluster_role: bool,
        /// The existing Role or ClusterRole to edit; a new one when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<ResourceRef>,
    },
    /// Create a Role or ClusterRole (E17-S05). **Mutating** (risk medium): blocked on a read-only
    /// cluster, one confirmation, a server dry run first, audited. Any other kind is refused.
    #[serde(rename = "role::Create")]
    RoleCreate {
        /// The cluster to create it in.
        cluster: ClusterId,
        /// The object as JSON text (`apiVersion: rbac.authorization.k8s.io/v1`).
        manifest: String,
    },
    /// Replace a Role or ClusterRole with an edited manifest (E17-S05). **Mutating** (risk
    /// medium). The manifest carries `metadata.resourceVersion`, so a change made since the
    /// dialog opened fails with a conflict instead of being overwritten.
    #[serde(rename = "role::Edit")]
    RoleEdit {
        /// The cluster the role is in.
        cluster: ClusterId,
        /// The whole object as JSON text, with its `resourceVersion`.
        manifest: String,
    },
    /// Delete a Role or ClusterRole (E17-S05). **Mutating** (risk medium).
    #[serde(rename = "role::Delete")]
    RoleDelete {
        /// The Role or ClusterRole.
        target: ResourceRef,
        /// How many bindings name it, when the caller counted them: the confirmation warns that
        /// they will be left pointing at nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bindings: Option<u32>,
    },
    /// Open the RoleBinding / ClusterRoleBinding dialog (E17-S06): a new binding, or the
    /// subjects and role of the existing `target`. Changes nothing; the dialog's save is
    /// [`BindingCreate`] or [`BindingEdit`].
    ///
    /// [`BindingCreate`]: Command::BindingCreate
    /// [`BindingEdit`]: Command::BindingEdit
    #[serde(rename = "binding::OpenDialog")]
    BindingOpenDialog {
        /// The cluster whose tab gets the dialog; the shown one when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cluster: Option<ClusterId>,
        /// A new ClusterRoleBinding instead of a new RoleBinding (ignored when `target` is set).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        cluster_binding: bool,
        /// The existing binding to edit; a new one when `None`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<ResourceRef>,
    },
    /// Create a RoleBinding or ClusterRoleBinding (E17-S06). **Mutating** (risk medium): blocked
    /// on a read-only cluster, one confirmation (a binding to `cluster-admin` or to everybody is
    /// called out), a server dry run first, audited. Any other kind is refused.
    #[serde(rename = "binding::Create")]
    BindingCreate {
        /// The cluster to create it in.
        cluster: ClusterId,
        /// The object as JSON text (`apiVersion: rbac.authorization.k8s.io/v1`).
        manifest: String,
    },
    /// Change a RoleBinding or ClusterRoleBinding (E17-S06). **Mutating** (risk medium).
    ///
    /// Without `previous` the manifest replaces the binding in place (its `roleRef` must be
    /// unchanged: the API refuses to change it, and `metadata.resourceVersion` makes a change
    /// made since the dialog opened fail with a conflict). With `previous` (the binding as it
    /// was) the `roleRef` changes, which the API only allows by making a new object: the old
    /// binding is deleted and the new one created as one audited intent, and the old one is put
    /// back if the new one is refused.
    #[serde(rename = "binding::Edit")]
    BindingEdit {
        /// The cluster the binding is in.
        cluster: ClusterId,
        /// The whole new object as JSON text.
        manifest: String,
        /// The binding as it was, as JSON text, when the `roleRef` changes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<String>,
    },
    /// Delete a RoleBinding or ClusterRoleBinding (E17-S06). **Mutating** (risk medium).
    #[serde(rename = "binding::Delete")]
    BindingDelete {
        /// The binding.
        target: ResourceRef,
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
            Command::ClusterShowMetricsSettings { .. } => CommandId::CLUSTER_SHOW_METRICS_SETTINGS,
            Command::HelmShowRepos => CommandId::HELM_SHOW_REPOS,
            Command::HelmOpenReleases { .. } => CommandId::HELM_OPEN_RELEASES,
            Command::RbacOpenUsers { .. } => CommandId::RBAC_OPEN_USERS,
            Command::RbacOpenGroups { .. } => CommandId::RBAC_OPEN_GROUPS,
            Command::RbacShowSubject { .. } => CommandId::RBAC_SHOW_SUBJECT,
            Command::RbacCopyMatrix { .. } => CommandId::RBAC_COPY_MATRIX,
            Command::HelmShowValues { .. } => CommandId::HELM_SHOW_VALUES,
            Command::HelmShowManifest { .. } => CommandId::HELM_SHOW_MANIFEST,
            Command::HelmRevealValues { .. } => CommandId::HELM_REVEAL_VALUES,
            Command::AuditOpen { .. } => CommandId::AUDIT_OPEN,
            Command::AuditExport { .. } => CommandId::AUDIT_EXPORT,
            Command::HelmOpenCharts => CommandId::HELM_OPEN_CHARTS,
            Command::HelmSearchCharts { .. } => CommandId::HELM_SEARCH_CHARTS,
            Command::HelmShowChart { .. } => CommandId::HELM_SHOW_CHART,
            Command::HelmInstallChart { .. } => CommandId::HELM_INSTALL_CHART,
            Command::HelmInstall { .. } => CommandId::HELM_INSTALL,
            Command::HelmUpgrade { .. } => CommandId::HELM_UPGRADE,
            Command::HelmUpgradeRelease { .. } => CommandId::HELM_UPGRADE_RELEASE,
            Command::HelmRollback { .. } => CommandId::HELM_ROLLBACK,
            Command::HelmUninstall { .. } => CommandId::HELM_UNINSTALL,
            Command::HelmRepoAdd { .. } => CommandId::HELM_REPO_ADD,
            Command::HelmRepoRemove { .. } => CommandId::HELM_REPO_REMOVE,
            Command::HelmRepoUpdate { .. } => CommandId::HELM_REPO_UPDATE,
            Command::ClusterShowPulses { .. } => CommandId::CLUSTER_SHOW_PULSES,
            Command::RbacCanI { .. } => CommandId::RBAC_CAN_I,
            Command::EventsOpen { .. } => CommandId::EVENTS_OPEN,
            Command::EventsToggleFaults { .. } => CommandId::EVENTS_TOGGLE_FAULTS,
            Command::EventsTogglePause { .. } => CommandId::EVENTS_TOGGLE_PAUSE,
            Command::EventsCopyRow { .. } => CommandId::EVENTS_COPY_ROW,
            Command::EventsCopyRowJson { .. } => CommandId::EVENTS_COPY_ROW_JSON,
            Command::EventsExportJson { .. } => CommandId::EVENTS_EXPORT_JSON,
            Command::ClusterShowOverview { .. } => CommandId::CLUSTER_SHOW_OVERVIEW,
            Command::ClusterRedetectMetrics { .. } => CommandId::CLUSTER_REDETECT_METRICS,
            Command::ClusterToggleMetrics { .. } => CommandId::CLUSTER_TOGGLE_METRICS,
            Command::EventsSendToAgent { .. } => CommandId::EVENTS_SEND_TO_AGENT,
            Command::ClusterApplyPreset { .. } => CommandId::CLUSTER_APPLY_PRESET,
            Command::NamespaceSelect { .. } => CommandId::NAMESPACE_SELECT,
            Command::NamespaceToggleFavourite { .. } => CommandId::NAMESPACE_TOGGLE_FAVOURITE,
            Command::NamespaceUseFavourite { .. } => CommandId::NAMESPACE_USE_FAVOURITE,
            Command::NamespaceCreate { .. } => CommandId::NAMESPACE_CREATE,
            Command::NamespaceDelete { .. } => CommandId::NAMESPACE_DELETE,
            Command::KubeconfigAddSource { .. } => CommandId::KUBECONFIG_ADD_SOURCE,
            Command::KubeconfigRemoveSource { .. } => CommandId::KUBECONFIG_REMOVE_SOURCE,
            Command::KubeconfigReload => CommandId::KUBECONFIG_RELOAD,
            Command::CloudRefresh { .. } => CommandId::CLOUD_REFRESH,
            Command::CloudCancel { .. } => CommandId::CLOUD_CANCEL,
            Command::CloudAdd { .. } => CommandId::CLOUD_ADD,
            Command::CloudRemove { .. } => CommandId::CLOUD_REMOVE,
            Command::ViewOpen { .. } => CommandId::VIEW_OPEN,
            Command::PaletteToggle => CommandId::PALETTE_TOGGLE,
            Command::PaletteClearRecents => CommandId::PALETTE_CLEAR_RECENTS,
            Command::PaletteOpenJump => CommandId::PALETTE_OPEN_JUMP,
            Command::HelpShow => CommandId::HELP_SHOW,
            Command::PaletteToggleShowAll => CommandId::PALETTE_TOGGLE_SHOW_ALL,
            Command::KeymapOpenUser => CommandId::KEYMAP_OPEN_USER,
            Command::ChartsOpenGallery => CommandId::CHARTS_OPEN_GALLERY,
            Command::AppQuit => CommandId::APP_QUIT,
            Command::AppSendTestNotification => CommandId::APP_SEND_TEST_NOTIFICATION,
            Command::AppToggleOsNotifications { .. } => CommandId::APP_TOGGLE_OS_NOTIFICATIONS,
            Command::HelmRecheck => CommandId::HELM_RECHECK,
            Command::HelmLocateBinary => CommandId::HELM_LOCATE_BINARY,
            Command::IngressClassSetDefault { .. } => CommandId::INGRESS_CLASS_SET_DEFAULT,
            Command::NotificationsTogglePanel => CommandId::NOTIFICATIONS_TOGGLE_PANEL,
            Command::NotificationsMarkRead { .. } => CommandId::NOTIFICATIONS_MARK_READ,
            Command::NotificationsMarkAllRead => CommandId::NOTIFICATIONS_MARK_ALL_READ,
            Command::NotificationsOpenLink { .. } => CommandId::NOTIFICATIONS_OPEN_LINK,
            Command::NotificationsMuteCluster { .. } => CommandId::NOTIFICATIONS_MUTE_CLUSTER,
            Command::NotificationsUnmuteCluster { .. } => CommandId::NOTIFICATIONS_UNMUTE_CLUSTER,
            Command::JumpBack => CommandId::JUMP_BACK,
            Command::JumpForward => CommandId::JUMP_FORWARD,
            Command::JumpLast => CommandId::JUMP_LAST,
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
            Command::TerminalClear => CommandId::TERMINAL_CLEAR,
            Command::TerminalScrollLineDown => CommandId::TERMINAL_SCROLL_LINE_DOWN,
            Command::TerminalScrollLineUp => CommandId::TERMINAL_SCROLL_LINE_UP,
            Command::TerminalScrollPageDown => CommandId::TERMINAL_SCROLL_PAGE_DOWN,
            Command::TerminalScrollPageUp => CommandId::TERMINAL_SCROLL_PAGE_UP,
            Command::TerminalSearch => CommandId::TERMINAL_SEARCH,
            Command::TerminalSearchClose => CommandId::TERMINAL_SEARCH_CLOSE,
            Command::TerminalSearchNext => CommandId::TERMINAL_SEARCH_NEXT,
            Command::TerminalSearchPrevious => CommandId::TERMINAL_SEARCH_PREVIOUS,
            Command::TerminalSelectAll => CommandId::TERMINAL_SELECT_ALL,
            Command::EditorNewManifest { .. } => CommandId::EDITOR_NEW_MANIFEST,
            Command::EditorToggleManagedFields => CommandId::EDITOR_TOGGLE_MANAGED_FIELDS,
            Command::EditorToggleSecretMask => CommandId::EDITOR_TOGGLE_SECRET_MASK,
            Command::EditorToggleReadOnly => CommandId::EDITOR_TOGGLE_READ_ONLY,
            Command::EditorToggleSoftWrap => CommandId::EDITOR_TOGGLE_SOFT_WRAP,
            Command::EditorConvertToJson => CommandId::EDITOR_CONVERT_TO_JSON,
            Command::EditorConvertToYaml => CommandId::EDITOR_CONVERT_TO_YAML,
            Command::EditorUseJson => CommandId::EDITOR_USE_JSON,
            Command::EditorUseYaml => CommandId::EDITOR_USE_YAML,
            Command::EditorShowDiff => CommandId::EDITOR_SHOW_DIFF,
            Command::EditorToggleDiffLayout => CommandId::EDITOR_TOGGLE_DIFF_LAYOUT,
            Command::EditorReloadFromServer => CommandId::EDITOR_RELOAD_FROM_SERVER,
            Command::EditorMergeFromServer => CommandId::EDITOR_MERGE_FROM_SERVER,
            Command::EditorKeepMine => CommandId::EDITOR_KEEP_MINE,
            Command::EditorTakeMine { .. } => CommandId::EDITOR_TAKE_MINE,
            Command::EditorTakeServer { .. } => CommandId::EDITOR_TAKE_SERVER,
            Command::EditorAcceptMerge => CommandId::EDITOR_ACCEPT_MERGE,
            Command::EditorTriggerCompletion => CommandId::EDITOR_TRIGGER_COMPLETION,
            Command::EditorShowHover => CommandId::EDITOR_SHOW_HOVER,
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
            Command::ResourceFind { .. } => CommandId::RESOURCE_FIND,
            Command::ResourceNextMatch { .. } => CommandId::RESOURCE_NEXT_MATCH,
            Command::ResourcePreviousMatch { .. } => CommandId::RESOURCE_PREVIOUS_MATCH,
            Command::ResourceRefreshDescribe { .. } => CommandId::RESOURCE_REFRESH_DESCRIBE,
            Command::ResourceSelectAll { .. } => CommandId::RESOURCE_SELECT_ALL,
            Command::ServiceOpenInBrowser { .. } => CommandId::SERVICE_OPEN_IN_BROWSER,
            Command::ServiceShowPods { .. } => CommandId::SERVICE_SHOW_PODS,
            Command::ResourceShowUsedBy { .. } => CommandId::RESOURCE_SHOW_USED_BY,
            Command::ResourceDiffWith { .. } => CommandId::RESOURCE_DIFF_WITH,
            Command::TableFocusFilter { .. } => CommandId::TABLE_FOCUS_FILTER,
            Command::TableSetFilter { .. } => CommandId::TABLE_SET_FILTER,
            Command::ResourceViewYaml { .. } => CommandId::RESOURCE_VIEW_YAML,
            Command::ResourceViewYamlInEditor { .. } => CommandId::RESOURCE_VIEW_YAML_IN_EDITOR,
            Command::ResourceViewDescribe { .. } => CommandId::RESOURCE_VIEW_DESCRIBE,
            Command::ResourceViewEvents { .. } => CommandId::RESOURCE_VIEW_EVENTS,
            Command::ResourceViewMetrics { .. } => CommandId::RESOURCE_VIEW_METRICS,
            Command::ResourceCreate { .. } => CommandId::RESOURCE_CREATE,
            Command::ResourceEdit { .. } => CommandId::RESOURCE_EDIT,
            Command::SecretReveal { .. } => CommandId::SECRET_REVEAL,
            Command::SecretEditDecoded { .. } => CommandId::SECRET_EDIT_DECODED,
            Command::SecretCreate { .. } => CommandId::SECRET_CREATE,
            Command::TableToggleWide { .. } => CommandId::TABLE_TOGGLE_WIDE,
            Command::ResourceDelete { .. } => CommandId::RESOURCE_DELETE,
            Command::ResourceApply { .. } => CommandId::RESOURCE_APPLY,
            Command::PodDelete { .. } => CommandId::POD_DELETE,
            Command::PodEvict { .. } => CommandId::POD_EVICT,
            Command::PodForceDelete { .. } => CommandId::POD_FORCE_DELETE,
            Command::PodSanitize { .. } => CommandId::POD_SANITIZE,
            Command::PodJumpToOwner { .. } => CommandId::POD_JUMP_TO_OWNER,
            Command::PodShowNode { .. } => CommandId::POD_SHOW_NODE,
            Command::PodShell { .. } => CommandId::POD_SHELL,
            Command::PodAttach { .. } => CommandId::POD_ATTACH,
            Command::PodExec { .. } => CommandId::POD_EXEC,
            Command::PodDebug { .. } => CommandId::POD_DEBUG,
            Command::PodPortForward { .. } => CommandId::POD_PORT_FORWARD,
            Command::PortForwardStart { .. } => CommandId::PORTFORWARD_START,
            Command::PortForwardStop { .. } => CommandId::PORTFORWARD_STOP,
            Command::PortForwardRestart { .. } => CommandId::PORTFORWARD_RESTART,
            Command::PortForwardRemove { .. } => CommandId::PORTFORWARD_REMOVE,
            Command::PortForwardFavouriteAdd { .. } => CommandId::PORTFORWARD_FAVOURITE_ADD,
            Command::PortForwardFavouriteRemove { .. } => CommandId::PORTFORWARD_FAVOURITE_REMOVE,
            Command::PortForwardFavouriteToggleAutoStart { .. } => {
                CommandId::PORTFORWARD_FAVOURITE_TOGGLE_AUTO_START
            }
            Command::PortForwardCopyUrl { .. } => CommandId::PORTFORWARD_COPY_URL,
            Command::PortForwardOpenInBrowser { .. } => CommandId::PORTFORWARD_OPEN_IN_BROWSER,
            Command::PortForwardTogglePanel { .. } => CommandId::PORTFORWARD_TOGGLE_PANEL,
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
            Command::WorkloadSetImage { .. } => CommandId::WORKLOAD_SET_IMAGE,
            Command::WorkloadRollback { .. } => CommandId::WORKLOAD_ROLLBACK,
            Command::WorkloadViewLogs { .. } => CommandId::WORKLOAD_VIEW_LOGS,
            Command::SelectionDelete { .. } => CommandId::SELECTION_DELETE,
            Command::SelectionRestart { .. } => CommandId::SELECTION_RESTART,
            Command::SelectionLabel { .. } => CommandId::SELECTION_LABEL,
            Command::NodeCordon { .. } => CommandId::NODE_CORDON,
            Command::NodeShell { .. } => CommandId::NODE_SHELL,
            Command::NodeUncordon { .. } => CommandId::NODE_UNCORDON,
            Command::CronJobTrigger { .. } => CommandId::CRONJOB_TRIGGER,
            Command::CronJobSuspend { .. } => CommandId::CRONJOB_SUSPEND,
            Command::CronJobResume { .. } => CommandId::CRONJOB_RESUME,
            Command::JobSuspend { .. } => CommandId::JOB_SUSPEND,
            Command::JobResume { .. } => CommandId::JOB_RESUME,
            Command::PvcResize { .. } => CommandId::PVC_RESIZE,
            Command::ServiceAccountGenerateKubeconfig { .. } => {
                CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG
            }
            Command::StorageClassSetDefault { .. } => CommandId::STORAGECLASS_SET_DEFAULT,
            Command::RoleOpenDialog { .. } => CommandId::ROLE_OPEN_DIALOG,
            Command::RoleCreate { .. } => CommandId::ROLE_CREATE,
            Command::RoleEdit { .. } => CommandId::ROLE_EDIT,
            Command::RoleDelete { .. } => CommandId::ROLE_DELETE,
            Command::BindingOpenDialog { .. } => CommandId::BINDING_OPEN_DIALOG,
            Command::BindingCreate { .. } => CommandId::BINDING_CREATE,
            Command::BindingEdit { .. } => CommandId::BINDING_EDIT,
            Command::BindingDelete { .. } => CommandId::BINDING_DELETE,
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

    /// The objects a bulk command acts on (`selection::*`); empty for every other command.
    pub fn targets(&self) -> &[ResourceRef] {
        match self {
            Command::SelectionDelete { targets, .. }
            | Command::SelectionRestart { targets }
            | Command::SelectionLabel { targets, .. } => targets,
            _ => &[],
        }
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
            | Command::ResourceFind { target, .. }
            | Command::ResourceNextMatch { target }
            | Command::ResourcePreviousMatch { target }
            | Command::ResourceRefreshDescribe { target }
            | Command::ResourceViewYaml { target }
            | Command::ResourceViewYamlInEditor { target }
            | Command::ResourceViewDescribe { target }
            | Command::ResourceViewEvents { target }
            | Command::ResourceViewMetrics { target }
            | Command::ResourceEdit { target }
            | Command::SecretReveal { target, .. }
            | Command::SecretEditDecoded { target }
            | Command::ResourceDelete { target, .. }
            | Command::PodDelete { target, .. }
            | Command::PodEvict { target }
            | Command::PodForceDelete { target }
            | Command::PodJumpToOwner { target }
            | Command::PodShowNode { target }
            | Command::PodShell { target, .. }
            | Command::PodAttach { target, .. }
            | Command::PodExec { target, .. }
            | Command::PodDebug { target, .. }
            | Command::PodPortForward { target, .. }
            | Command::PortForwardStart { target, .. }
            | Command::PortForwardFavouriteAdd { target, .. }
            | Command::ServiceOpenInBrowser { target, .. }
            | Command::ServiceShowPods { target }
            | Command::ResourceShowUsedBy { target }
            | Command::ResourceDiffWith { target, .. }
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
            | Command::WorkloadSetImage { target, .. }
            | Command::WorkloadRollback { target, .. }
            | Command::WorkloadViewLogs { target, .. }
            | Command::NamespaceDelete { target, .. }
            | Command::NodeCordon { target }
            | Command::NodeShell { target }
            | Command::NodeUncordon { target }
            | Command::IngressClassSetDefault { target, .. }
            | Command::NodeDrain { target, .. }
            | Command::CronJobTrigger { target }
            | Command::CronJobSuspend { target }
            | Command::CronJobResume { target }
            | Command::JobSuspend { target }
            | Command::JobResume { target }
            | Command::PvcResize { target, .. }
            | Command::RoleDelete { target, .. }
            | Command::ServiceAccountGenerateKubeconfig { target, .. }
            | Command::BindingDelete { target }
            | Command::StorageClassSetDefault { target, .. } => Some(target),
            Command::RoleOpenDialog { target, .. } | Command::BindingOpenDialog { target, .. } => {
                target.as_ref()
            }
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

    fn service() -> ResourceRef {
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Service"), "default", "web")
    }

    fn deployment() -> ResourceRef {
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("apps", "v1", "Deployment"),
            "prod",
            "api",
        )
    }

    fn cronjob() -> ResourceRef {
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("batch", "v1", "CronJob"),
            "default",
            "nightly",
        )
    }

    fn job() -> ResourceRef {
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("batch", "v1", "Job"),
            "default",
            "import",
        )
    }

    fn node() -> ResourceRef {
        ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), "worker-1")
    }

    fn ingress_class() -> ResourceRef {
        ResourceRef::cluster_scoped(
            cluster(),
            Gvk::new("networking.k8s.io", "v1", "IngressClass"),
            "nginx",
        )
    }

    fn claim() -> ResourceRef {
        ResourceRef::namespaced(
            cluster(),
            Gvk::new("", "v1", "PersistentVolumeClaim"),
            "shop",
            "data",
        )
    }

    fn storage_class() -> ResourceRef {
        ResourceRef::cluster_scoped(
            cluster(),
            Gvk::new("storage.k8s.io", "v1", "StorageClass"),
            "fast",
        )
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
            Command::ClusterShowMetricsSettings { cluster: cluster() },
            Command::HelmShowRepos,
            Command::HelmOpenReleases { cluster: cluster() },
            Command::HelmShowValues {
                cluster: cluster(),
                namespace: "apps".into(),
                name: "web".into(),
                revision: Some(3),
                all: true,
            },
            Command::HelmShowManifest {
                cluster: cluster(),
                namespace: "apps".into(),
                name: "web".into(),
                revision: None,
            },
            Command::HelmRevealValues {
                cluster: cluster(),
                namespace: "apps".into(),
                name: "web".into(),
                reveal: true,
            },
            Command::HelmOpenCharts,
            Command::AuditOpen { cluster: None },
            Command::AuditOpen {
                cluster: Some(cluster()),
            },
            Command::AuditExport {
                format: AuditExportFormat::Jsonl,
            },
            Command::AuditExport {
                format: AuditExportFormat::Csv,
            },
            Command::HelmSearchCharts {
                keyword: "nginx".into(),
            },
            Command::HelmShowChart {
                repo: "bitnami".into(),
                name: "nginx".into(),
                version: Some("15.4.0".into()),
                part: crate::helm::ChartPart::Values,
            },
            Command::HelmShowChart {
                repo: "bitnami".into(),
                name: "nginx".into(),
                version: None,
                part: crate::helm::ChartPart::Readme,
            },
            Command::HelmInstallChart {
                repo: "bitnami".into(),
                name: "nginx".into(),
                version: None,
            },
            Command::HelmUpgradeRelease {
                namespace: Some("demo".into()),
                release: Some("web".into()),
            },
            Command::HelmUpgradeRelease {
                namespace: None,
                release: None,
            },
            Command::HelmUpgrade {
                cluster: cluster(),
                namespace: "demo".into(),
                release: "web".into(),
                repo: Some("bitnami".into()),
                chart: "nginx".into(),
                version: Some("15.5.0".into()),
                values: crate::helm::ValuesYaml::new("replicaCount: 3\n"),
                reuse_values: true,
                wait: true,
                timeout_secs: Some(120),
            },
            Command::HelmInstall {
                cluster: cluster(),
                release: "web".into(),
                namespace: "demo".into(),
                create_namespace: true,
                repo: Some("bitnami".into()),
                chart: "nginx".into(),
                version: Some("15.4.0".into()),
                values: crate::helm::ValuesYaml::new("replicaCount: 2\n"),
                wait: true,
                timeout_secs: Some(120),
            },
            Command::HelmInstall {
                cluster: cluster(),
                release: "web".into(),
                namespace: "demo".into(),
                create_namespace: false,
                repo: None,
                chart: "oci://example.com/nginx".into(),
                version: None,
                values: crate::helm::ValuesYaml::default(),
                wait: false,
                timeout_secs: None,
            },
            Command::HelmRollback {
                cluster: cluster(),
                namespace: "demo".into(),
                release: "web".into(),
                revision: 2,
            },
            Command::HelmUninstall {
                cluster: cluster(),
                namespace: "demo".into(),
                release: "web".into(),
                keep_history: false,
            },
            Command::HelmUninstall {
                cluster: cluster(),
                namespace: "demo".into(),
                release: "web".into(),
                keep_history: true,
            },
            Command::HelmRepoAdd {
                name: "bitnami".into(),
                url: "https://charts.bitnami.com/bitnami".into(),
                username: Some("me".into()),
                insecure_skip_tls_verify: true,
                ca_file: Some("/etc/ca.pem".into()),
            },
            Command::HelmRepoAdd {
                name: "bitnami".into(),
                url: "https://charts.bitnami.com/bitnami".into(),
                username: None,
                insecure_skip_tls_verify: false,
                ca_file: None,
            },
            Command::HelmRepoRemove {
                name: "bitnami".into(),
            },
            Command::HelmRepoUpdate { names: vec![] },
            Command::HelmRepoUpdate {
                names: vec!["bitnami".into()],
            },
            Command::ClusterShowPulses { cluster: cluster() },
            Command::RbacCanI {
                cluster: cluster(),
                target: None,
                verb: None,
            },
            Command::RbacCanI {
                cluster: cluster(),
                target: Some(ResourceRef::namespaced(
                    cluster(),
                    Gvk::new("", "v1", "Pod"),
                    "shop",
                    "web-0",
                )),
                verb: Some("delete".into()),
            },
            Command::EventsOpen {
                cluster: cluster(),
                namespace: None,
                filter: None,
            },
            Command::EventsOpen {
                cluster: cluster(),
                namespace: Some("kube-system".into()),
                filter: Some("BackOff".into()),
            },
            Command::RbacOpenUsers { cluster: cluster() },
            Command::RbacOpenGroups { cluster: cluster() },
            Command::RbacShowSubject {
                cluster: cluster(),
                subject: "system:serviceaccount:default:builder".into(),
            },
            Command::RbacCopyMatrix { cluster: cluster() },
            Command::EventsToggleFaults { cluster: cluster() },
            Command::EventsTogglePause { cluster: cluster() },
            Command::EventsCopyRow { cluster: cluster() },
            Command::EventsCopyRowJson { cluster: cluster() },
            Command::EventsExportJson { cluster: cluster() },
            Command::ClusterShowOverview { cluster: cluster() },
            Command::ClusterRedetectMetrics { cluster: cluster() },
            Command::ClusterToggleMetrics {
                cluster: cluster(),
                enabled: None,
            },
            Command::EventsSendToAgent {
                cluster: cluster(),
                target: Some(pod()),
                events: Vec::new(),
            },
            Command::EventsSendToAgent {
                cluster: cluster(),
                target: None,
                events: vec!["uid-1".into()],
            },
            Command::ClusterToggleMetrics {
                cluster: cluster(),
                enabled: Some(false),
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
            Command::NamespaceUseFavourite {
                cluster: cluster(),
                slot: 2,
            },
            Command::NamespaceCreate {
                cluster: cluster(),
                name: "team-a".into(),
                labels: BTreeMap::from([("team".to_owned(), "a".to_owned())]),
            },
            Command::NamespaceDelete {
                target: ResourceRef::cluster_scoped(
                    cluster(),
                    Gvk::new("", "v1", "Namespace"),
                    "team-a",
                ),
                contents: BTreeMap::from([("Pod".to_owned(), 12)]),
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
            Command::CloudRefresh {
                provider: Some(CloudProvider::Aws),
                mode: CloudRefreshMode::Force,
            },
            Command::CloudCancel { provider: None },
            Command::CloudAdd {
                provider: CloudProvider::Gcp,
                cluster: "gcp/p/europe-west1/c".into(),
            },
            Command::CloudRemove {
                provider: CloudProvider::Azure,
                context: ContextName::new("aks-sub-westeurope-c"),
            },
            Command::ViewOpen {
                view: "overview".into(),
            },
            Command::PaletteToggle,
            Command::PaletteClearRecents,
            Command::PaletteOpenJump,
            Command::HelpShow,
            Command::PaletteToggleShowAll,
            Command::KeymapOpenUser,
            Command::ChartsOpenGallery,
            Command::AppQuit,
            Command::AppSendTestNotification,
            Command::AppToggleOsNotifications { enabled: None },
            Command::HelmRecheck,
            Command::HelmLocateBinary,
            Command::NotificationsTogglePanel,
            Command::NotificationsMarkRead { id: 7 },
            Command::NotificationsMarkAllRead,
            Command::NotificationsOpenLink {
                url: "https://example.com/release".to_owned(),
            },
            Command::NotificationsMuteCluster { cluster: cluster() },
            Command::NotificationsUnmuteCluster { cluster: cluster() },
            Command::JumpBack,
            Command::JumpForward,
            Command::JumpLast,
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
            Command::TerminalClear,
            Command::TerminalScrollLineDown,
            Command::TerminalScrollLineUp,
            Command::TerminalScrollPageDown,
            Command::TerminalScrollPageUp,
            Command::TerminalSearch,
            Command::TerminalSearchClose,
            Command::TerminalSearchNext,
            Command::TerminalSearchPrevious,
            Command::TerminalSelectAll,
            Command::EditorNewManifest { cluster: None },
            Command::EditorNewManifest {
                cluster: Some(cluster()),
            },
            Command::ResourceCreate {
                cluster: None,
                template: None,
            },
            Command::ResourceCreate {
                cluster: Some(cluster()),
                template: Some("Deployment".into()),
            },
            Command::EditorToggleManagedFields,
            Command::EditorToggleSecretMask,
            Command::EditorToggleReadOnly,
            Command::EditorToggleSoftWrap,
            Command::EditorConvertToJson,
            Command::EditorConvertToYaml,
            Command::EditorUseJson,
            Command::EditorUseYaml,
            Command::EditorShowDiff,
            Command::EditorToggleDiffLayout,
            Command::EditorReloadFromServer,
            Command::EditorMergeFromServer,
            Command::EditorKeepMine,
            Command::EditorTakeMine { conflict: 0 },
            Command::EditorTakeServer { conflict: 2 },
            Command::EditorAcceptMerge,
            Command::EditorTriggerCompletion,
            Command::EditorShowHover,
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
            Command::ResourceFind {
                target: pod(),
                pattern: Some("image".into()),
            },
            Command::ResourceNextMatch { target: pod() },
            Command::ResourcePreviousMatch { target: pod() },
            Command::ResourceRefreshDescribe { target: pod() },
            Command::ResourceSelectAll {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::ServiceOpenInBrowser {
                target: service(),
                port: Some(80),
            },
            Command::ServiceShowPods { target: service() },
            Command::ResourceShowUsedBy { target: pod() },
            Command::ResourceDiffWith {
                target: pod(),
                other: Some(pod()),
                include_status: true,
                compare_identity: false,
            },
            Command::TableFocusFilter {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::TableSetFilter {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
                text: "/api -l app=x".to_owned(),
            },
            Command::ResourceViewYaml { target: pod() },
            Command::ResourceViewYamlInEditor { target: pod() },
            Command::ResourceViewDescribe { target: pod() },
            Command::ResourceViewEvents { target: pod() },
            Command::ResourceViewMetrics { target: pod() },
            Command::ResourceEdit { target: pod() },
            Command::SecretReveal {
                target: pod(),
                key: Some("password".into()),
            },
            Command::SecretEditDecoded { target: pod() },
            Command::SecretCreate {
                cluster: Some(cluster()),
            },
            Command::RoleOpenDialog {
                cluster: Some(cluster()),
                cluster_role: true,
                target: None,
            },
            Command::BindingOpenDialog {
                cluster: Some(cluster()),
                cluster_binding: true,
                target: None,
            },
            Command::BindingCreate {
                cluster: cluster(),
                manifest: "{}".into(),
            },
            Command::BindingEdit {
                cluster: cluster(),
                manifest: "{}".into(),
                previous: Some("{}".into()),
            },
            Command::BindingDelete { target: pod() },
            Command::RoleCreate {
                cluster: cluster(),
                manifest: "{}".into(),
            },
            Command::RoleEdit {
                cluster: cluster(),
                manifest: "{}".into(),
            },
            Command::RoleDelete {
                target: pod(),
                bindings: Some(2),
            },
            Command::TableToggleWide {
                cluster: cluster(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::ResourceDelete {
                target: deployment(),
                propagation: Propagation::Foreground,
            },
            Command::ResourceApply {
                cluster: cluster(),
                namespace: Some("default".into()),
                manifest: "kind: ConfigMap".into(),
                field_manager: Some("oxikube".into()),
                force: true,
                resource_version: Some("42".into()),
                reviewed_absent: true,
            },
            Command::PodDelete {
                target: pod(),
                grace_period_seconds: Some(0),
            },
            Command::PodEvict { target: pod() },
            Command::PodForceDelete { target: pod() },
            Command::PodSanitize {
                cluster: cluster(),
                namespace: Some("default".into()),
                targets: vec![pod()],
            },
            Command::PodJumpToOwner { target: pod() },
            Command::PodShowNode { target: pod() },
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
            Command::PodDebug {
                target: pod(),
                image: "busybox".into(),
                target_container: Some("app".into()),
                command: vec!["sh".into()],
                name: None,
            },
            Command::PodPortForward {
                target: pod(),
                local_port: None,
                remote_port: 8080,
            },
            Command::PortForwardStart {
                target: pod(),
                remote_port: ForwardPort::Number(8080),
                local_port: Some(18080),
                bind: None,
                name: None,
                favourite: false,
            },
            Command::PortForwardStop { id: 1 },
            Command::PortForwardRestart { id: 1 },
            Command::PortForwardRemove { id: 1 },
            Command::PortForwardFavouriteAdd {
                target: pod(),
                remote_port: ForwardPort::Named("http".into()),
                local_port: None,
                bind: None,
                name: Some("db".into()),
                auto_start: Some(false),
            },
            Command::PortForwardFavouriteRemove { id: "fav-1".into() },
            Command::PortForwardFavouriteToggleAutoStart { id: "fav-1".into() },
            Command::PortForwardCopyUrl { id: 1 },
            Command::PortForwardOpenInBrowser { id: 1 },
            Command::PortForwardTogglePanel { show: false },
            Command::PortForwardTogglePanel { show: true },
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
            Command::SelectionDelete {
                targets: vec![pod()],
                propagation: Propagation::Background,
            },
            Command::SelectionRestart {
                targets: vec![pod()],
            },
            Command::SelectionLabel {
                targets: vec![pod()],
                field: LabelField::Labels,
                edits: vec![LabelEdit::set("app", "web"), LabelEdit::remove("old")],
            },
            Command::WorkloadRestart {
                target: deployment(),
            },
            Command::WorkloadSetImage {
                target: deployment(),
                images: vec![ImageChange::new("web", "nginx:1.27")],
            },
            Command::WorkloadRollback {
                target: deployment(),
                to_revision: Some(2),
            },
            Command::NodeCordon { target: node() },
            Command::NodeShell { target: node() },
            Command::NodeUncordon { target: node() },
            Command::NodeDrain {
                target: node(),
                flags: crate::drain::DrainFlags {
                    force: true,
                    ..crate::drain::DrainFlags::default()
                },
            },
            Command::IngressClassSetDefault {
                target: ingress_class(),
                clear_previous: true,
            },
            Command::CronJobTrigger { target: cronjob() },
            Command::CronJobSuspend { target: cronjob() },
            Command::CronJobResume { target: cronjob() },
            Command::JobSuspend { target: job() },
            Command::JobResume { target: job() },
            Command::PvcResize {
                target: claim(),
                size: "20Gi".into(),
                from: Some("10Gi".into()),
            },
            Command::StorageClassSetDefault {
                target: storage_class(),
                clear_previous: true,
            },
            Command::ServiceAccountGenerateKubeconfig {
                target: ResourceRef::namespaced(
                    cluster(),
                    Gvk::new("", "v1", "ServiceAccount"),
                    "demo",
                    "deployer",
                ),
                expiration_seconds: Some(3600),
                audiences: vec!["https://example.com".into()],
                allow_legacy_token: true,
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
    fn a_debug_command_defaults_to_busybox_and_is_a_mutation_not_exec_class() {
        let debug: Command = serde_json::from_value(json!({
            "type": "pod::Debug",
            "target": pod(),
        }))
        .unwrap();
        assert_eq!(
            debug,
            Command::PodDebug {
                target: pod(),
                image: DEFAULT_DEBUG_IMAGE.into(),
                target_container: None,
                command: Vec::new(),
                name: None,
            }
        );
        assert!(debug.is_mutating() && !debug.is_exec());
        assert_eq!(debug.effective_risk(), Some(crate::safety::Risk::Low));
        assert_eq!(debug.target(), Some(&pod()));
        assert!(debug.meta().interactive);
        // What the dialog sends survives a round trip unchanged.
        let full = Command::PodDebug {
            target: pod(),
            image: "nicolaka/netshoot".into(),
            target_container: Some("app".into()),
            command: vec!["bash".into()],
            name: Some("dbg".into()),
        };
        let back: Command = serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back, full);
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
                    | Command::NamespaceUseFavourite { .. }
                    | Command::ViewOpen { .. }
                    | Command::PaletteToggle
                    | Command::PaletteClearRecents
                    | Command::PaletteOpenJump
                    | Command::HelpShow
                    | Command::PaletteToggleShowAll
                    | Command::KeymapOpenUser
                    | Command::ChartsOpenGallery
                    | Command::AppQuit
                    | Command::AppSendTestNotification
                    | Command::AppToggleOsNotifications { .. }
                    | Command::HelmRecheck
                    | Command::HelmLocateBinary
                    | Command::NotificationsTogglePanel
                    | Command::NotificationsMarkRead { .. }
                    | Command::NotificationsMarkAllRead
                    | Command::NotificationsOpenLink { .. }
                    | Command::NotificationsMuteCluster { .. }
                    | Command::NotificationsUnmuteCluster { .. }
                    | Command::JumpBack
                    | Command::JumpForward
                    | Command::JumpLast
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
                    | Command::TerminalClear
                    | Command::TerminalScrollLineDown
                    | Command::TerminalScrollLineUp
                    | Command::TerminalScrollPageDown
                    | Command::TerminalScrollPageUp
                    | Command::TerminalSearch
                    | Command::TerminalSearchClose
                    | Command::TerminalSearchNext
                    | Command::TerminalSearchPrevious
                    | Command::TerminalSelectAll
                    | Command::EditorNewManifest { .. }
                    | Command::ResourceCreate { .. }
                    | Command::EditorToggleManagedFields
                    | Command::EditorToggleSecretMask
                    | Command::EditorToggleReadOnly
                    | Command::EditorToggleSoftWrap
                    | Command::EditorConvertToJson
                    | Command::EditorConvertToYaml
                    | Command::EditorUseJson
                    | Command::EditorUseYaml
                    | Command::EditorShowDiff
                    | Command::EditorToggleDiffLayout
                    | Command::EditorReloadFromServer
                    | Command::EditorMergeFromServer
                    | Command::EditorKeepMine
                    | Command::EditorTakeMine { .. }
                    | Command::EditorTakeServer { .. }
                    | Command::EditorAcceptMerge
                    | Command::EditorTriggerCompletion
                    | Command::EditorShowHover
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
                    | Command::ResourceFind { .. }
                    | Command::ResourceNextMatch { .. }
                    | Command::ResourcePreviousMatch { .. }
                    | Command::ResourceRefreshDescribe { .. }
                    | Command::ResourceSelectAll { .. }
                    | Command::ServiceOpenInBrowser { .. }
                    | Command::PortForwardStart { .. }
                    | Command::PortForwardStop { .. }
                    | Command::PortForwardRestart { .. }
                    | Command::PortForwardRemove { .. }
                    | Command::PortForwardFavouriteAdd { .. }
                    | Command::PortForwardCopyUrl { .. }
                    | Command::PortForwardOpenInBrowser { .. }
                    | Command::PortForwardTogglePanel { .. }
                    | Command::ServiceShowPods { .. }
                    | Command::ResourceShowUsedBy { .. }
                    | Command::ResourceDiffWith { .. }
                    | Command::TableFocusFilter { .. }
                    | Command::TableSetFilter { .. }
                    | Command::ResourceViewYaml { .. }
                    | Command::ResourceViewYamlInEditor { .. }
                    | Command::ResourceViewDescribe { .. }
                    | Command::ResourceViewEvents { .. }
                    | Command::ResourceViewMetrics { .. }
                    | Command::ResourceEdit { .. }
                    | Command::SecretReveal { .. }
                    | Command::SecretEditDecoded { .. }
                    | Command::SecretCreate { .. }
                    | Command::RoleOpenDialog { .. }
                    | Command::BindingOpenDialog { .. }
                    | Command::TableToggleWide { .. }
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
                    | Command::ClusterShowMetricsSettings { .. }
                    | Command::HelmShowRepos
                    | Command::HelmOpenReleases { .. }
                    | Command::RbacOpenUsers { .. }
                    | Command::RbacOpenGroups { .. }
                    | Command::RbacShowSubject { .. }
                    | Command::RbacCopyMatrix { .. }
                    | Command::HelmShowValues { .. }
                    | Command::HelmShowManifest { .. }
                    | Command::HelmRevealValues { .. }
                    | Command::HelmOpenCharts
                    | Command::AuditOpen { .. }
                    | Command::AuditExport { .. }
                    | Command::HelmSearchCharts { .. }
                    | Command::HelmShowChart { .. }
                    | Command::HelmInstallChart { .. }
                    | Command::HelmUpgradeRelease { .. }
                    | Command::HelmRepoAdd { .. }
                    | Command::HelmRepoRemove { .. }
                    | Command::HelmRepoUpdate { .. }
                    | Command::ClusterShowPulses { .. }
                    | Command::RbacCanI { .. }
                    | Command::EventsOpen { .. }
                    | Command::EventsToggleFaults { .. }
                    | Command::EventsTogglePause { .. }
                    | Command::EventsCopyRow { .. }
                    | Command::EventsCopyRowJson { .. }
                    | Command::EventsExportJson { .. }
                    | Command::ClusterShowOverview { .. }
                    | Command::ClusterRedetectMetrics { .. }
                    | Command::ClusterToggleMetrics { .. }
                    | Command::EventsSendToAgent { .. }
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
    fn cloud_commands_are_flat_json_app_tools_without_a_guard_tier() {
        let command: Command = serde_json::from_value(json!({
            "type": "cloud::Refresh",
            "provider": "gcp",
        }))
        .unwrap();
        assert_eq!(
            command,
            Command::CloudRefresh {
                provider: Some(CloudProvider::Gcp),
                mode: CloudRefreshMode::IfStale,
            }
        );
        assert_eq!(command.id().tool_name(), "app.cloud_refresh");
        assert_eq!(
            serde_json::to_value(Command::CloudCancel { provider: None }).unwrap(),
            json!({ "type": "cloud::Cancel" })
        );
        for command in [
            command,
            Command::CloudCancel { provider: None },
            Command::CloudAdd {
                provider: CloudProvider::Aws,
                cluster: "aws/p/eu-west-1/c".into(),
            },
            Command::CloudRemove {
                provider: CloudProvider::Aws,
                context: ContextName::new("eks-p-eu-west-1-c"),
            },
        ] {
            let meta = command.meta();
            assert!(!meta.mutating && !meta.privileged, "{}", meta.id);
            assert_eq!(meta.confirm, crate::safety::ConfirmTier::None);
        }
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
