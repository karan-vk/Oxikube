//! The static registry of declared commands: [`COMMANDS`], [`lookup`] and the
//! [`CommandId`] constants.
//!
//! One entry per [`Command`](super::Command) variant. Adding a command means a
//! new constant here, a registry row (keep the slice sorted by id), a variant
//! in `payload.rs`, and a handler in `oxikube_app`. The tests in this module
//! catch an unsorted slice, a duplicate id, a malformed id, a colliding tool
//! name and a mutating command without a deliberate confirmation tier.

use super::availability::{SelectionKind, ViewContext};
use super::capability::Capabilities;
use super::id::CommandId;
use super::meta::{CommandMeta, CommandScope};
use crate::safety::Risk;

impl CommandId {
    /// `app::Quit`: quit the application (confirms first while operations run).
    pub const APP_QUIT: CommandId = CommandId::new("app::Quit");
    /// `helm::Recheck`: detect the `helm` binary again and refresh what the Helm views offer.
    pub const HELM_RECHECK: CommandId = CommandId::new("helm::Recheck");
    /// `helm::LocateBinary`: open the `helm.path` setting.
    pub const HELM_LOCATE_BINARY: CommandId = CommandId::new("helm::LocateBinary");
    /// `app::SendTestNotification`: show one OS notification, to check that they reach the desktop.
    pub const APP_SEND_TEST_NOTIFICATION: CommandId = CommandId::new("app::SendTestNotification");
    /// `app::ToggleOsNotifications`: turn desktop (OS) notifications on or off (the opt-in).
    pub const APP_TOGGLE_OS_NOTIFICATIONS: CommandId = CommandId::new("app::ToggleOsNotifications");
    /// `charts::OpenGallery`: open the chart gallery, every chart state on one page (a reference
    /// page for the metrics views and their reviewers).
    pub const CHARTS_OPEN_GALLERY: CommandId = CommandId::new("charts::OpenGallery");
    /// `cluster::ApplyPreset`: give a cluster a prod / staging / dev / none posture.
    pub const CLUSTER_APPLY_PRESET: CommandId = CommandId::new("cluster::ApplyPreset");
    /// `cluster::CancelConnect`: cancel a connection attempt that is still in flight.
    pub const CLUSTER_CANCEL_CONNECT: CommandId = CommandId::new("cluster::CancelConnect");
    /// `cluster::CloseTab`: close a cluster's tab (disconnects it).
    pub const CLUSTER_CLOSE_TAB: CommandId = CommandId::new("cluster::CloseTab");
    /// `cluster::Connect`: connect a cluster (open its session).
    pub const CLUSTER_CONNECT: CommandId = CommandId::new("cluster::Connect");
    /// `cluster::Disconnect`: disconnect a cluster or cancel the attempt.
    pub const CLUSTER_DISCONNECT: CommandId = CommandId::new("cluster::Disconnect");
    /// `cluster::NextTab`: show the next cluster tab.
    pub const CLUSTER_NEXT_TAB: CommandId = CommandId::new("cluster::NextTab");
    /// `cluster::PreviousTab`: show the previous cluster tab.
    pub const CLUSTER_PREVIOUS_TAB: CommandId = CommandId::new("cluster::PreviousTab");
    /// `cluster::Reconnect`: drop a cluster's connection and connect it again (the retry).
    pub const CLUSTER_RECONNECT: CommandId = CommandId::new("cluster::Reconnect");
    /// `cluster::RedetectMetrics`: run metrics provider detection for a cluster again.
    pub const CLUSTER_REDETECT_METRICS: CommandId = CommandId::new("cluster::RedetectMetrics");
    /// `cluster::Select`: make a cluster the active one.
    pub const CLUSTER_SELECT: CommandId = CommandId::new("cluster::Select");
    /// `cluster::SetColour`: set or clear a cluster's accent colour.
    pub const CLUSTER_SET_COLOUR: CommandId = CommandId::new("cluster::SetColour");
    /// `cluster::ShowMetricsSettings`: open a cluster's metrics settings page.
    pub const CLUSTER_SHOW_METRICS_SETTINGS: CommandId =
        CommandId::new("cluster::ShowMetricsSettings");
    /// `cluster::ShowPulses`: open a cluster's pulses page.
    pub const CLUSTER_SHOW_PULSES: CommandId = CommandId::new("cluster::ShowPulses");
    /// `cluster::ShowOverview`: open a cluster's overview page.
    pub const CLUSTER_SHOW_OVERVIEW: CommandId = CommandId::new("cluster::ShowOverview");
    /// `cluster::SwitchTab`: show the nth cluster tab.
    pub const CLUSTER_SWITCH_TAB: CommandId = CommandId::new("cluster::SwitchTab");
    /// `cluster::ToggleFavourite`: mark or unmark a cluster as a favourite.
    pub const CLUSTER_TOGGLE_FAVOURITE: CommandId = CommandId::new("cluster::ToggleFavourite");
    /// `cluster::ToggleMetrics`: switch a cluster's history metrics on or off.
    pub const CLUSTER_TOGGLE_METRICS: CommandId = CommandId::new("cluster::ToggleMetrics");
    /// `cluster::ToggleReadOnly`: set or toggle a cluster's read-only mode.
    pub const CLUSTER_TOGGLE_READ_ONLY: CommandId = CommandId::new("cluster::ToggleReadOnly");
    /// `cronjob::Resume`: let a suspended CronJob create Jobs again.
    pub const CRONJOB_RESUME: CommandId = CommandId::new("cronjob::Resume");
    /// `cronjob::Suspend`: stop a CronJob creating Jobs.
    pub const CRONJOB_SUSPEND: CommandId = CommandId::new("cronjob::Suspend");
    /// `cronjob::Trigger`: run a CronJob now by creating a Job from its template.
    pub const CRONJOB_TRIGGER: CommandId = CommandId::new("cronjob::Trigger");
    /// `crd::OpenList`: open the list of the cluster's CustomResourceDefinitions.
    pub const CRD_OPEN_LIST: CommandId = CommandId::new("crd::OpenList");
    /// `crd::OpenResources`: open the table of the custom resources a CRD defines.
    pub const CRD_OPEN_RESOURCES: CommandId = CommandId::new("crd::OpenResources");
    /// `editor::AcceptMerge`: put the focused editor's settled merge in its buffer.
    pub const EDITOR_ACCEPT_MERGE: CommandId = CommandId::new("editor::AcceptMerge");
    /// `editor::KeepMine`: keep the focused editor's buffer over the server's change.
    pub const EDITOR_KEEP_MINE: CommandId = CommandId::new("editor::KeepMine");
    /// `editor::MergeFromServer`: merge the server's change into the focused editor's buffer.
    pub const EDITOR_MERGE_FROM_SERVER: CommandId = CommandId::new("editor::MergeFromServer");
    /// `editor::ReloadFromServer`: replace the focused editor's buffer with the live object.
    pub const EDITOR_RELOAD_FROM_SERVER: CommandId = CommandId::new("editor::ReloadFromServer");
    /// `editor::ShowDiff`: show or hide the focused editor's diff against the live object.
    pub const EDITOR_SHOW_DIFF: CommandId = CommandId::new("editor::ShowDiff");
    /// `editor::TakeMine`: settle a merge conflict with the buffer's lines.
    pub const EDITOR_TAKE_MINE: CommandId = CommandId::new("editor::TakeMine");
    /// `editor::TakeServer`: settle a merge conflict with the server's lines.
    pub const EDITOR_TAKE_SERVER: CommandId = CommandId::new("editor::TakeServer");
    /// `editor::ToggleDiffLayout`: switch the editor's diff between side by side and inline.
    pub const EDITOR_TOGGLE_DIFF_LAYOUT: CommandId = CommandId::new("editor::ToggleDiffLayout");
    /// `editor::ConvertToJson`: replace the focused manifest editor's YAML with the same data as JSON.
    pub const EDITOR_CONVERT_TO_JSON: CommandId = CommandId::new("editor::ConvertToJson");
    /// `editor::ConvertToYaml`: replace the focused manifest editor's JSON with the same data as YAML.
    pub const EDITOR_CONVERT_TO_YAML: CommandId = CommandId::new("editor::ConvertToYaml");
    /// `editor::NewManifest`: open an empty manifest editor tab.
    pub const EDITOR_NEW_MANIFEST: CommandId = CommandId::new("editor::NewManifest");
    /// `editor::ToggleManagedFields`: show or hide `metadata.managedFields` in the focused
    /// manifest editor of a resource.
    pub const EDITOR_TOGGLE_MANAGED_FIELDS: CommandId =
        CommandId::new("editor::ToggleManagedFields");
    /// `editor::ToggleReadOnly`: make the focused manifest editor read-only, or editable again.
    pub const EDITOR_TOGGLE_READ_ONLY: CommandId = CommandId::new("editor::ToggleReadOnly");
    /// `editor::ToggleSecretMask`: show or hide the values of a Secret in the focused manifest
    /// editor of a resource.
    pub const EDITOR_TOGGLE_SECRET_MASK: CommandId = CommandId::new("editor::ToggleSecretMask");
    /// `editor::ToggleSoftWrap`: wrap the focused manifest editor's long lines, or not.
    pub const EDITOR_TOGGLE_SOFT_WRAP: CommandId = CommandId::new("editor::ToggleSoftWrap");
    /// `editor::UseJson`: read the focused manifest editor's buffer as JSON (highlighting and checks).
    pub const EDITOR_USE_JSON: CommandId = CommandId::new("editor::UseJson");
    /// `editor::UseYaml`: read the focused manifest editor's buffer as YAML (highlighting and checks).
    pub const EDITOR_USE_YAML: CommandId = CommandId::new("editor::UseYaml");
    /// `editor::ShowHover`: show the schema docs of the token at the cursor.
    pub const EDITOR_SHOW_HOVER: CommandId = CommandId::new("editor::ShowHover");
    /// `editor::TriggerCompletion`: open the completion menu at the cursor.
    pub const EDITOR_TRIGGER_COMPLETION: CommandId = CommandId::new("editor::TriggerCompletion");
    /// `events::SendToAgent`: queue the selected events (or an object's events) as context for the
    /// hosted agent.
    pub const EVENTS_SEND_TO_AGENT: CommandId = CommandId::new("events::SendToAgent");
    /// `events::CopyRow`: copy the selected row of the events view as text.
    pub const EVENTS_COPY_ROW: CommandId = CommandId::new("events::CopyRow");
    /// `events::CopyRowJson`: copy the selected event of the events view as JSON.
    pub const EVENTS_COPY_ROW_JSON: CommandId = CommandId::new("events::CopyRowJson");
    /// `events::ExportJson`: write the events the events view lists to a JSON file.
    pub const EVENTS_EXPORT_JSON: CommandId = CommandId::new("events::ExportJson");
    /// `events::Open`: open a cluster's events view (`:ev`).
    pub const EVENTS_OPEN: CommandId = CommandId::new("events::Open");
    /// `events::ToggleFaults`: show only `Warning` events in the events view, or every event.
    pub const EVENTS_TOGGLE_FAULTS: CommandId = CommandId::new("events::ToggleFaults");
    /// `events::TogglePause`: pause the events view's live tail, or resume it.
    pub const EVENTS_TOGGLE_PAUSE: CommandId = CommandId::new("events::TogglePause");
    /// `help::Show`: show the key bindings that apply where the focus is (the `?` overlay).
    pub const HELP_SHOW: CommandId = CommandId::new("help::Show");
    /// `ingressclass::SetDefault`: make an IngressClass the cluster's default (annotation patch).
    pub const INGRESS_CLASS_SET_DEFAULT: CommandId = CommandId::new("ingressclass::SetDefault");
    /// `jump::Back`: run the previous command of the `:` jump bar's history again.
    pub const JUMP_BACK: CommandId = CommandId::new("jump::Back");
    /// `jump::Forward`: run the next command of the `:` jump bar's history again.
    pub const JUMP_FORWARD: CommandId = CommandId::new("jump::Forward");
    /// `jump::Last`: go back to the view before the current one (`-` in the jump bar).
    pub const JUMP_LAST: CommandId = CommandId::new("jump::Last");
    /// `job::Resume`: resume a suspended Job.
    pub const JOB_RESUME: CommandId = CommandId::new("job::Resume");
    /// `job::Suspend`: suspend a Job (its pods are deleted until it is resumed).
    pub const JOB_SUSPEND: CommandId = CommandId::new("job::Suspend");
    /// `keymap::OpenUser`: open the user's `keymap.json`, creating it from a commented template
    /// first when it does not exist.
    pub const KEYMAP_OPEN_USER: CommandId = CommandId::new("keymap::OpenUser");
    /// `kubeconfig::AddSource`: add a kubeconfig file, directory or pasted text as a source.
    pub const KUBECONFIG_ADD_SOURCE: CommandId = CommandId::new("kubeconfig::AddSource");
    /// `kubeconfig::Reload`: re-read every kubeconfig source.
    pub const KUBECONFIG_RELOAD: CommandId = CommandId::new("kubeconfig::Reload");
    /// `kubeconfig::RemoveSource`: remove a kubeconfig source.
    pub const KUBECONFIG_REMOVE_SOURCE: CommandId = CommandId::new("kubeconfig::RemoveSource");
    /// `logs::CloseSearch`: close a log view's search bar and clear its highlights and filter.
    pub const LOGS_CLOSE_SEARCH: CommandId = CommandId::new("logs::CloseSearch");
    /// `logs::Find`: open a log view's search bar (optionally with a pattern).
    pub const LOGS_FIND: CommandId = CommandId::new("logs::Find");
    /// `logs::NextMatch`: go to the next match of a log view's search (wraps around).
    pub const LOGS_NEXT_MATCH: CommandId = CommandId::new("logs::NextMatch");
    /// `logs::PreviousMatch`: go to the previous match of a log view's search (wraps around).
    pub const LOGS_PREVIOUS_MATCH: CommandId = CommandId::new("logs::PreviousMatch");
    /// `logs::ToggleCase`: make a log view's search case-sensitive, or not.
    pub const LOGS_TOGGLE_CASE: CommandId = CommandId::new("logs::ToggleCase");
    /// `logs::ToggleFilterMode`: hide the lines that do not match a log view's search, or show all
    /// of them with the matches highlighted.
    pub const LOGS_TOGGLE_FILTER_MODE: CommandId = CommandId::new("logs::ToggleFilterMode");
    /// `logs::ToggleInverse`: match the lines that do not contain a log view's pattern, or those
    /// that do.
    pub const LOGS_TOGGLE_INVERSE: CommandId = CommandId::new("logs::ToggleInverse");
    /// `logs::Clear`: empty a log view's local buffer (the cluster's logs are untouched).
    pub const LOGS_CLEAR: CommandId = CommandId::new("logs::Clear");
    /// `logs::Copy`: copy the selected lines of a log view (else what is on screen).
    pub const LOGS_COPY: CommandId = CommandId::new("logs::Copy");
    /// `logs::Mark`: mark or unmark the focused line of a log view.
    pub const LOGS_MARK: CommandId = CommandId::new("logs::Mark");
    /// `logs::SendToAgent`: queue the selected lines of a log view as context for the hosted agent.
    pub const LOGS_SEND_TO_AGENT: CommandId = CommandId::new("logs::SendToAgent");
    /// `logs::TailInTerminal`: run `kubectl logs -f` for a log view's pod or workload in a terminal
    /// tab of the cluster (the power-user fallback; needs kubectl on this machine).
    pub const LOGS_TAIL_IN_TERMINAL: CommandId = CommandId::new("logs::TailInTerminal");
    /// `logs::Save`: save a log view's lines to a file the user picks.
    pub const LOGS_SAVE: CommandId = CommandId::new("logs::Save");
    /// `logs::FollowReplacement`: switch a log view whose pod was replaced (a rollout, a
    /// StatefulSet's recreated pod) to the pod that took over.
    pub const LOGS_FOLLOW_REPLACEMENT: CommandId = CommandId::new("logs::FollowReplacement");
    /// `logs::Reconnect`: open a log view's stream again after it failed or ended, keeping the
    /// lines it holds.
    pub const LOGS_RECONNECT: CommandId = CommandId::new("logs::Reconnect");
    /// `logs::SelectContainer`: show another container of a log view's pod (reopens the stream).
    pub const LOGS_SELECT_CONTAINER: CommandId = CommandId::new("logs::SelectContainer");
    /// `logs::SetRange`: read the tail, the head or the last minutes of a log view's log.
    pub const LOGS_SET_RANGE: CommandId = CommandId::new("logs::SetRange");
    /// `logs::ToggleAutoscroll`: follow the newest line of a log view, or stop following.
    pub const LOGS_TOGGLE_AUTOSCROLL: CommandId = CommandId::new("logs::ToggleAutoscroll");
    /// `logs::ToggleJsonMode`: show structured lines as columns, or every line as raw text.
    pub const LOGS_TOGGLE_JSON_MODE: CommandId = CommandId::new("logs::ToggleJsonMode");
    /// `logs::ToggleLevel`: show or hide the lines of one level in a log view.
    pub const LOGS_TOGGLE_LEVEL: CommandId = CommandId::new("logs::ToggleLevel");
    /// `logs::ToggleLine`: expand a structured line into its pretty-printed pane, or close it.
    pub const LOGS_TOGGLE_LINE: CommandId = CommandId::new("logs::ToggleLine");
    /// `logs::CollapseLine`: close the expanded-line pane of a log view.
    pub const LOGS_COLLAPSE_LINE: CommandId = CommandId::new("logs::CollapseLine");
    /// `logs::ToggleFullscreen`: let a log view fill its cluster tab, or give the space back.
    pub const LOGS_TOGGLE_FULLSCREEN: CommandId = CommandId::new("logs::ToggleFullscreen");
    /// `logs::TogglePrevious`: read the previous (terminated) container instance, or the current.
    pub const LOGS_TOGGLE_PREVIOUS: CommandId = CommandId::new("logs::TogglePrevious");
    /// `logs::ToggleSource`: show or hide one pod's (or container's) lines in a multi-pod log view.
    pub const LOGS_TOGGLE_SOURCE: CommandId = CommandId::new("logs::ToggleSource");
    /// `logs::ToggleTimestamps`: show or hide the server timestamps of a log view.
    pub const LOGS_TOGGLE_TIMESTAMPS: CommandId = CommandId::new("logs::ToggleTimestamps");
    /// `logs::ToggleWrap`: wrap a log view's long lines, or let them run off the edge.
    pub const LOGS_TOGGLE_WRAP: CommandId = CommandId::new("logs::ToggleWrap");
    /// `namespace::Create`: create a namespace, with optional labels.
    pub const NAMESPACE_CREATE: CommandId = CommandId::new("namespace::Create");
    /// `namespace::Delete`: delete a namespace and everything in it.
    pub const NAMESPACE_DELETE: CommandId = CommandId::new("namespace::Delete");
    /// `namespace::Select`: choose the namespace selection.
    pub const NAMESPACE_SELECT: CommandId = CommandId::new("namespace::Select");
    /// `namespace::ToggleFavourite`: pin or unpin a namespace as a favourite.
    pub const NAMESPACE_TOGGLE_FAVOURITE: CommandId = CommandId::new("namespace::ToggleFavourite");
    /// `namespace::UseFavourite`: select all namespaces (`0`) or the nth favourite (`1`-`9`).
    pub const NAMESPACE_USE_FAVOURITE: CommandId = CommandId::new("namespace::UseFavourite");
    /// `node::Cordon`: mark a node unschedulable.
    pub const NODE_CORDON: CommandId = CommandId::new("node::Cordon");
    /// `node::Drain`: evict a node's pods.
    pub const NODE_DRAIN: CommandId = CommandId::new("node::Drain");
    /// `node::Shell`: open a shell on a node through a privileged pod.
    pub const NODE_SHELL: CommandId = CommandId::new("node::Shell");
    /// `node::Uncordon`: mark a node schedulable again.
    pub const NODE_UNCORDON: CommandId = CommandId::new("node::Uncordon");
    /// `notifications::MarkAllRead`: mark every notification of the centre read.
    pub const NOTIFICATIONS_MARK_ALL_READ: CommandId = CommandId::new("notifications::MarkAllRead");
    /// `notifications::MarkRead`: mark one notification of the centre read.
    pub const NOTIFICATIONS_MARK_READ: CommandId = CommandId::new("notifications::MarkRead");
    /// `notifications::MuteCluster`: stop notifying about a cluster's events (remembered).
    pub const NOTIFICATIONS_MUTE_CLUSTER: CommandId = CommandId::new("notifications::MuteCluster");
    /// `notifications::OpenLink`: open a notification's `https` link in the browser.
    pub const NOTIFICATIONS_OPEN_LINK: CommandId = CommandId::new("notifications::OpenLink");
    /// `notifications::TogglePanel`: show and focus the notification centre, or hide it.
    pub const NOTIFICATIONS_TOGGLE_PANEL: CommandId = CommandId::new("notifications::TogglePanel");
    /// `notifications::UnmuteCluster`: notify about a muted cluster's events again.
    pub const NOTIFICATIONS_UNMUTE_CLUSTER: CommandId =
        CommandId::new("notifications::UnmuteCluster");
    /// `palette::ClearRecents`: forget the commands the palette lists first because they ran
    /// lately.
    pub const PALETTE_CLEAR_RECENTS: CommandId = CommandId::new("palette::ClearRecents");
    /// `palette::OpenJump`: open the `:` jump bar.
    pub const PALETTE_OPEN_JUMP: CommandId = CommandId::new("palette::OpenJump");
    /// `palette::Toggle`: show or hide the command palette.
    pub const PALETTE_TOGGLE: CommandId = CommandId::new("palette::Toggle");
    /// `palette::ToggleShowAll`: list the commands that cannot run here too, marked, in the open
    /// command palette.
    pub const PALETTE_TOGGLE_SHOW_ALL: CommandId = CommandId::new("palette::ToggleShowAll");
    /// `pod::Attach`: attach to a container's main process.
    pub const POD_ATTACH: CommandId = CommandId::new("pod::Attach");
    /// `pod::Debug`: add an ephemeral debug container to a pod and open a terminal in it.
    pub const POD_DEBUG: CommandId = CommandId::new("pod::Debug");
    /// `pod::Delete`: delete one pod.
    pub const POD_DELETE: CommandId = CommandId::new("pod::Delete");
    /// `pod::Evict`: evict one pod through the eviction subresource (honours PodDisruptionBudgets).
    pub const POD_EVICT: CommandId = CommandId::new("pod::Evict");
    /// `pod::Exec`: run a command (or shell) in a container.
    pub const POD_EXEC: CommandId = CommandId::new("pod::Exec");
    /// `pod::ForceDelete`: delete one pod with grace period 0 (k9s's kill).
    pub const POD_FORCE_DELETE: CommandId = CommandId::new("pod::ForceDelete");
    /// `pod::JumpToOwner`: open the controller that owns a pod.
    pub const POD_JUMP_TO_OWNER: CommandId = CommandId::new("pod::JumpToOwner");
    /// `pod::PortForward`: forward a local port to a pod port.
    pub const POD_PORT_FORWARD: CommandId = CommandId::new("pod::PortForward");
    /// `portforward::Start`: start a port-forward to a pod or a Service (E15-S03).
    pub const PORTFORWARD_START: CommandId = CommandId::new("portforward::Start");
    /// `portforward::Stop`: stop a port-forward, keeping it listed (E15-S03).
    pub const PORTFORWARD_STOP: CommandId = CommandId::new("portforward::Stop");
    /// `portforward::Restart`: start a port-forward again (E15-S03).
    pub const PORTFORWARD_RESTART: CommandId = CommandId::new("portforward::Restart");
    /// `portforward::Remove`: stop a port-forward and drop it from the list (E15-S03).
    pub const PORTFORWARD_REMOVE: CommandId = CommandId::new("portforward::Remove");
    /// `portforward::FavouriteAdd`: save a port-forward as a favourite (E15-S06).
    pub const PORTFORWARD_FAVOURITE_ADD: CommandId = CommandId::new("portforward::FavouriteAdd");
    /// `portforward::FavouriteRemove`: remove a favourite (E15-S06).
    pub const PORTFORWARD_FAVOURITE_REMOVE: CommandId =
        CommandId::new("portforward::FavouriteRemove");
    /// `portforward::FavouriteToggleAutoStart`: switch a favourite's start on connect (E15-S06).
    pub const PORTFORWARD_FAVOURITE_TOGGLE_AUTO_START: CommandId =
        CommandId::new("portforward::FavouriteToggleAutoStart");
    /// `portforward::CopyUrl`: copy a port-forward's local URL (E15-S05).
    pub const PORTFORWARD_COPY_URL: CommandId = CommandId::new("portforward::CopyUrl");
    /// `portforward::OpenInBrowser`: open a port-forward's local URL in the system browser
    /// (E15-S05).
    pub const PORTFORWARD_OPEN_IN_BROWSER: CommandId = CommandId::new("portforward::OpenInBrowser");
    /// `portforward::TogglePanel`: show the Port Forwards panel, or hide it (E15-S05).
    pub const PORTFORWARD_TOGGLE_PANEL: CommandId = CommandId::new("portforward::TogglePanel");
    /// `pod::Sanitize`: delete the Completed and Failed pods of a view.
    pub const POD_SANITIZE: CommandId = CommandId::new("pod::Sanitize");
    /// `pod::ShowNode`: open the Node a pod runs on.
    pub const POD_SHOW_NODE: CommandId = CommandId::new("pod::ShowNode");
    /// `pod::Shell`: open an interactive shell in a container (`bash`, else `sh`).
    pub const POD_SHELL: CommandId = CommandId::new("pod::Shell");
    /// `pod::ViewLogs`: open a pod's logs.
    pub const POD_VIEW_LOGS: CommandId = CommandId::new("pod::ViewLogs");
    /// `pvc::Resize`: grow a PersistentVolumeClaim (patch its requested storage).
    pub const PVC_RESIZE: CommandId = CommandId::new("pvc::Resize");
    /// `resource::Apply`: apply a manifest.
    pub const RESOURCE_APPLY: CommandId = CommandId::new("resource::Apply");
    /// `resource::CopyLabel`: copy a label or annotation of a resource as `key=value`.
    pub const RESOURCE_COPY_LABEL: CommandId = CommandId::new("resource::CopyLabel");
    /// `resource::CopyName`: copy a resource's name to the clipboard.
    pub const RESOURCE_COPY_NAME: CommandId = CommandId::new("resource::CopyName");
    /// `resource::CopyYaml`: copy the YAML a resource's detail shows (secrets masked).
    pub const RESOURCE_COPY_YAML: CommandId = CommandId::new("resource::CopyYaml");
    /// `resource::Create`: open the "Create resource" picker (or one template) in the manifest
    /// editor.
    pub const RESOURCE_CREATE: CommandId = CommandId::new("resource::Create");
    /// `resource::Delete`: delete any resource.
    pub const RESOURCE_DELETE: CommandId = CommandId::new("resource::Delete");
    /// `resource::Edit`: open a resource's manifest in the editor.
    pub const RESOURCE_EDIT: CommandId = CommandId::new("resource::Edit");
    /// `resource::Open`: open a resource's detail view.
    pub const RESOURCE_OPEN: CommandId = CommandId::new("resource::Open");
    /// `resource::OpenList`: open the list view of a resource kind.
    pub const RESOURCE_OPEN_LIST: CommandId = CommandId::new("resource::OpenList");
    /// `resource::RefreshDescribe`: read a resource's describe text again.
    pub const RESOURCE_REFRESH_DESCRIBE: CommandId = CommandId::new("resource::RefreshDescribe");
    /// `resource::RetryFeed`: restart the feed behind a kind's list views.
    pub const RESOURCE_RETRY_FEED: CommandId = CommandId::new("resource::RetryFeed");
    /// `resource::SaveYaml`: save the YAML a resource's detail shows to a file (secrets masked).
    pub const RESOURCE_SAVE_YAML: CommandId = CommandId::new("resource::SaveYaml");
    /// `resource::PinDetail`: promote a resource's detail drawer to a workspace tab.
    pub const RESOURCE_PIN_DETAIL: CommandId = CommandId::new("resource::PinDetail");
    /// `resource::SelectAll`: select every row of a kind's list views.
    pub const RESOURCE_SELECT_ALL: CommandId = CommandId::new("resource::SelectAll");
    /// `resource::ToggleManagedFields`: show or hide `metadata.managedFields` in a resource's YAML.
    pub const RESOURCE_TOGGLE_MANAGED_FIELDS: CommandId =
        CommandId::new("resource::ToggleManagedFields");
    /// `resource::Find`: open the find field of a resource detail's YAML or Describe text,
    /// optionally with the pattern already typed.
    pub const RESOURCE_FIND: CommandId = CommandId::new("resource::Find");
    /// `resource::NextMatch`: go to the next match of a resource detail's find, wrapping.
    pub const RESOURCE_NEXT_MATCH: CommandId = CommandId::new("resource::NextMatch");
    /// `resource::PreviousMatch`: go to the previous match of a resource detail's find, wrapping.
    pub const RESOURCE_PREVIOUS_MATCH: CommandId = CommandId::new("resource::PreviousMatch");
    /// `resource::ViewDescribe`: open a resource's `kubectl describe` text.
    pub const RESOURCE_VIEW_DESCRIBE: CommandId = CommandId::new("resource::ViewDescribe");
    /// `resource::ViewEvents`: open a resource's detail on its Events tab.
    pub const RESOURCE_VIEW_EVENTS: CommandId = CommandId::new("resource::ViewEvents");
    /// `resource::ViewMetrics`: open a resource's detail on its Metrics tab (history graphs).
    pub const RESOURCE_VIEW_METRICS: CommandId = CommandId::new("resource::ViewMetrics");
    /// `resource::ViewYaml`: open a resource's YAML.
    pub const RESOURCE_VIEW_YAML: CommandId = CommandId::new("resource::ViewYaml");
    /// `resource::ViewYamlInEditor`: open a resource's YAML read-only in a manifest editor tab.
    pub const RESOURCE_VIEW_YAML_IN_EDITOR: CommandId =
        CommandId::new("resource::ViewYamlInEditor");
    /// `service::OpenInBrowser`: forward a Service port and open it in the browser.
    pub const SERVICE_OPEN_IN_BROWSER: CommandId = CommandId::new("service::OpenInBrowser");
    /// `service::ShowPods`: open the pods a Service selects, in a table filtered by its selector.
    pub const SERVICE_SHOW_PODS: CommandId = CommandId::new("service::ShowPods");
    /// `storageclass::SetDefault`: make a StorageClass the cluster's default.
    pub const STORAGECLASS_SET_DEFAULT: CommandId = CommandId::new("storageclass::SetDefault");
    /// `resource::ShowUsedBy`: open a ServiceAccount, ConfigMap, Secret, claim or PriorityClass on
    /// its "Used by" tab (k9s `u`).
    pub const RESOURCE_SHOW_USED_BY: CommandId = CommandId::new("resource::ShowUsedBy");
    /// `secret::Create`: open the "Create secret" dialog (Opaque, Docker registry, TLS).
    pub const SECRET_CREATE: CommandId = CommandId::new("secret::Create");
    /// `secret::EditDecoded`: open a Secret in the manifest editor with its values decoded.
    pub const SECRET_EDIT_DECODED: CommandId = CommandId::new("secret::EditDecoded");
    /// `secret::Reveal`: show, or hide again, a Secret's values decoded in its detail (k9s `x`).
    pub const SECRET_REVEAL: CommandId = CommandId::new("secret::Reveal");
    /// `selection::Delete`: delete the selected objects as one batch, one confirmation at the
    /// riskiest object's tier, a result per object (E12-S19).
    pub const SELECTION_DELETE: CommandId = CommandId::new("selection::Delete");
    /// `selection::Label`: add, change or remove labels or annotations on the selected objects as
    /// one batch (E12-S19).
    pub const SELECTION_LABEL: CommandId = CommandId::new("selection::Label");
    /// `selection::Restart`: rolling-restart the selected Deployments, StatefulSets and DaemonSets
    /// as one batch (E12-S19).
    pub const SELECTION_RESTART: CommandId = CommandId::new("selection::Restart");
    /// `table::FocusFilter`: move the keyboard focus to a resource table's filter bar.
    pub const TABLE_FOCUS_FILTER: CommandId = CommandId::new("table::FocusFilter");
    /// `table::ToggleWide`: show or hide the wide columns (`kubectl -o wide`) of a kind's tables.
    pub const TABLE_TOGGLE_WIDE: CommandId = CommandId::new("table::ToggleWide");
    /// `table::SetFilter`: put a filter in a resource table's filter bar and apply it.
    pub const TABLE_SET_FILTER: CommandId = CommandId::new("table::SetFilter");
    /// `terminal::Close`: close the focused terminal, ending its process.
    pub const TERMINAL_CLOSE: CommandId = CommandId::new("terminal::Close");
    /// `terminal::Clear`: clear the focused terminal's scrollback and the screen above the cursor.
    pub const TERMINAL_CLEAR: CommandId = CommandId::new("terminal::Clear");
    /// `terminal::Copy`: copy the focused terminal's selection to the clipboard.
    pub const TERMINAL_COPY: CommandId = CommandId::new("terminal::Copy");
    /// `terminal::New`: open a local shell in a new terminal of the (displayed) cluster's tab.
    pub const TERMINAL_NEW: CommandId = CommandId::new("terminal::New");
    /// `terminal::OpenLink`: open a URL or local path a terminal shows.
    pub const TERMINAL_OPEN_LINK: CommandId = CommandId::new("terminal::OpenLink");
    /// `terminal::Paste`: paste the clipboard into the focused terminal.
    pub const TERMINAL_PASTE: CommandId = CommandId::new("terminal::Paste");
    /// `terminal::Reconnect`: open the focused pod terminal's session again after the connection
    /// dropped (a new session in the same container).
    pub const TERMINAL_RECONNECT: CommandId = CommandId::new("terminal::Reconnect");
    /// `terminal::Restart`: start a new shell in the focused local terminal after its shell exited.
    pub const TERMINAL_RESTART: CommandId = CommandId::new("terminal::Restart");
    /// `terminal::ScrollLineDown`: scroll the focused terminal one line towards the live screen.
    pub const TERMINAL_SCROLL_LINE_DOWN: CommandId = CommandId::new("terminal::ScrollLineDown");
    /// `terminal::ScrollLineUp`: scroll the focused terminal one line into its history.
    pub const TERMINAL_SCROLL_LINE_UP: CommandId = CommandId::new("terminal::ScrollLineUp");
    /// `terminal::ScrollPageDown`: scroll the focused terminal one screen towards the live screen.
    pub const TERMINAL_SCROLL_PAGE_DOWN: CommandId = CommandId::new("terminal::ScrollPageDown");
    /// `terminal::ScrollPageUp`: scroll the focused terminal one screen into its history.
    pub const TERMINAL_SCROLL_PAGE_UP: CommandId = CommandId::new("terminal::ScrollPageUp");
    /// `terminal::Search`: open the focused terminal's search bar.
    pub const TERMINAL_SEARCH: CommandId = CommandId::new("terminal::Search");
    /// `terminal::SearchClose`: close the focused terminal's search bar.
    pub const TERMINAL_SEARCH_CLOSE: CommandId = CommandId::new("terminal::SearchClose");
    /// `terminal::SearchNext`: jump to the next match of the focused terminal's search.
    pub const TERMINAL_SEARCH_NEXT: CommandId = CommandId::new("terminal::SearchNext");
    /// `terminal::SearchPrevious`: jump to the previous match of the focused terminal's search.
    pub const TERMINAL_SEARCH_PREVIOUS: CommandId = CommandId::new("terminal::SearchPrevious");
    /// `terminal::SelectAll`: select the focused terminal's screen and scrollback.
    pub const TERMINAL_SELECT_ALL: CommandId = CommandId::new("terminal::SelectAll");
    /// `terminal::Split`: open a new terminal in a new pane beside the active one.
    pub const TERMINAL_SPLIT: CommandId = CommandId::new("terminal::Split");
    /// `view::Open`: open a registered view by id.
    pub const VIEW_OPEN: CommandId = CommandId::new("view::Open");
    /// `view::ZoomIn`: make the UI one zoom step larger.
    pub const VIEW_ZOOM_IN: CommandId = CommandId::new("view::ZoomIn");
    /// `view::ZoomOut`: make the UI one zoom step smaller.
    pub const VIEW_ZOOM_OUT: CommandId = CommandId::new("view::ZoomOut");
    /// `view::ZoomReset`: set the UI zoom back to 100 %.
    pub const VIEW_ZOOM_RESET: CommandId = CommandId::new("view::ZoomReset");
    /// `window::New`: open another main window.
    pub const WINDOW_NEW: CommandId = CommandId::new("window::New");
    /// `workload::Restart`: rolling-restart a workload.
    pub const WORKLOAD_RESTART: CommandId = CommandId::new("workload::Restart");
    /// `workload::Rollback`: restore a workload's pod template from an earlier revision.
    pub const WORKLOAD_ROLLBACK: CommandId = CommandId::new("workload::Rollback");
    /// `workload::Scale`: set a workload's replica count.
    pub const WORKLOAD_SCALE: CommandId = CommandId::new("workload::Scale");
    /// `workload::SetImage`: change the image of containers of a workload's pod template.
    pub const WORKLOAD_SET_IMAGE: CommandId = CommandId::new("workload::SetImage");
    /// `workload::ViewLogs`: open the merged logs of a workload's or a Service's pods.
    pub const WORKLOAD_VIEW_LOGS: CommandId = CommandId::new("workload::ViewLogs");
}

const NONE: Capabilities = Capabilities::empty();

/// The views a selection of resources is acted on from: a table row or the detail view of one.
const TABLE_VIEWS: &[ViewContext] = &[ViewContext::Table, ViewContext::Detail];
/// The kinds a restart rolls: the pod-template workloads whose controller replaces pods when the
/// template changes (a ReplicaSet and a ReplicationController never do).
/// What `portforward::Start` forwards to: a pod, or the pods behind a Service.
const FORWARDABLE_KINDS: &[(&str, &str)] = &[("", "Pod"), ("", "Service")];

const ROLLABLE_KINDS: &[(&str, &str)] = &[
    ("apps", "Deployment"),
    ("apps", "StatefulSet"),
    ("apps", "DaemonSet"),
];
/// Commands of the resource table only.
const TABLE_VIEW: &[ViewContext] = &[ViewContext::Table];
/// Commands of the detail view only.
const DETAIL_VIEW: &[ViewContext] = &[ViewContext::Detail];
/// Commands of the log viewer.
const LOG_VIEW: &[ViewContext] = &[ViewContext::Logs];
/// Commands of a terminal.
const TERMINAL_VIEW: &[ViewContext] = &[ViewContext::Terminal];
/// Commands of the manifest editor.
const EDITOR_VIEW: &[ViewContext] = &[ViewContext::Editor];
/// Commands of the events view.
const EVENTS_VIEW: &[ViewContext] = &[ViewContext::Events];

/// Every declared command, **sorted by id** (lookup is a binary search; a test
/// enforces the order).
pub static COMMANDS: &[CommandMeta] = &[
    CommandMeta::read(
        CommandId::APP_QUIT,
        "Quit Oxikube",
        CommandScope::Global,
        NONE,
    ),
    // Shows one notification through the OS notifier (only when the user opted in); changes nothing
    // in any cluster and carries no cluster data.
    CommandMeta::read(
        CommandId::APP_SEND_TEST_NOTIFICATION,
        "Send Test OS Notification",
        CommandScope::Global,
        NONE,
    ),
    // Edits the user's own `events.os_notifications` setting (the opt-in); not a cluster mutation,
    // so it runs on a read-only cluster.
    CommandMeta::read(
        CommandId::APP_TOGGLE_OS_NOTIFICATIONS,
        "Toggle OS Notifications",
        CommandScope::Global,
        NONE,
    ),
    // Opens the chart gallery (a page of every chart state) as a tab; reads nothing from a
    // cluster and changes nothing.
    CommandMeta::read(
        CommandId::CHARTS_OPEN_GALLERY,
        "Open Chart Gallery",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_APPLY_PRESET,
        "Apply Cluster Preset",
        CommandScope::Cluster,
        NONE,
    ),
    // Gives up an attempt in flight; nothing is read or changed in the cluster.
    CommandMeta::read(
        CommandId::CLUSTER_CANCEL_CONNECT,
        "Cancel Connecting",
        CommandScope::Global,
        NONE,
    ),
    // Closing a tab disconnects: it reads nothing and changes nothing in the cluster, so it is no
    // mutation. The confirmation it may show (running operations) is a UI prompt, not a guard tier.
    CommandMeta::read(
        CommandId::CLUSTER_CLOSE_TAB,
        "Close Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    // Connecting reads from the cluster and changes nothing in it: not `mutating`, no guard tier.
    CommandMeta::read(
        CommandId::CLUSTER_CONNECT,
        "Connect Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_DISCONNECT,
        "Disconnect Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_NEXT_TAB,
        "Next Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_PREVIOUS_TAB,
        "Previous Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    // Retrying a connection reads from the cluster like `cluster::Connect`: no guard tier.
    CommandMeta::read(
        CommandId::CLUSTER_RECONNECT,
        "Reconnect Cluster",
        CommandScope::Global,
        NONE,
    ),
    // Lists Services and sends one probe query through the API server; changes nothing in the
    // cluster.
    CommandMeta::read(
        CommandId::CLUSTER_REDETECT_METRICS,
        "Look for Metrics Provider Again",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_SELECT,
        "Select Cluster",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_SET_COLOUR,
        "Set Cluster Colour",
        CommandScope::Cluster,
        NONE,
    ),
    // Opens a page; reads the session's own state and nothing from the cluster.
    CommandMeta::read(
        CommandId::CLUSTER_SHOW_METRICS_SETTINGS,
        "Cluster Metrics Settings",
        CommandScope::Cluster,
        NONE,
    ),
    // Opens a page; reads what the session's store and the metrics services already hold.
    CommandMeta::read(
        CommandId::CLUSTER_SHOW_OVERVIEW,
        "Cluster Overview",
        CommandScope::Cluster,
        NONE,
    ),
    // Opens a page; the page counts what the stores already watch and reads metrics-server, and
    // changes nothing in the cluster.
    CommandMeta::read(
        CommandId::CLUSTER_SHOW_PULSES,
        "Show Pulses",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CLUSTER_SWITCH_TAB,
        "Switch Cluster Tab",
        CommandScope::Global,
        NONE,
    ),
    // Local catalog preference (StatePort), never a cluster change.
    CommandMeta::read(
        CommandId::CLUSTER_TOGGLE_FAVOURITE,
        "Toggle Favourite Cluster",
        CommandScope::Global,
        NONE,
    ),
    // A local preference (`prometheus.enabled` in the user's settings), never a cluster change.
    CommandMeta::read(
        CommandId::CLUSTER_TOGGLE_METRICS,
        "Toggle Cluster Metrics",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::privileged(
        CommandId::CLUSTER_TOGGLE_READ_ONLY,
        "Toggle Read-Only Mode",
        CommandScope::Cluster,
        NONE,
    ),
    // The CRD navigation commands read the cluster to find a kind's list; they open views and
    // change nothing in it.
    CommandMeta::read(
        CommandId::CRD_OPEN_LIST,
        "Open Custom Resource Definitions",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::CRD_OPEN_RESOURCES,
        "Open Custom Resources",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::OfKind {
        group: "apiextensions.k8s.io",
        kind: "CustomResourceDefinition",
    }),
    // E12-S08. Low risk (a simple confirm): a trigger creates one Job the CronJob owns, a suspend
    // or resume flips `spec.suspend` and is undone by its sibling. `kubectl create job --from`
    // and a merge patch; neither is a cascade.
    CommandMeta::mutation(
        CommandId::CRONJOB_RESUME,
        "Resume CronJob",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::OfKind {
        group: "batch",
        kind: "CronJob",
    }),
    CommandMeta::mutation(
        CommandId::CRONJOB_SUSPEND,
        "Suspend CronJob",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::OfKind {
        group: "batch",
        kind: "CronJob",
    }),
    CommandMeta::mutation(
        CommandId::CRONJOB_TRIGGER,
        "Trigger CronJob",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::OfKind {
        group: "batch",
        kind: "CronJob",
    }),
    // The manifest editor (E10-S04) only edits text in this window: a new buffer and the view's
    // own toggles read and change nothing in a cluster. What reaches the cluster goes through
    // `resource::Apply`, which is guarded.
    // The conversions replace the buffer's text (one undo step) and read nothing from a cluster;
    // the mode switches change only how the buffer is highlighted and checked (E10-S10).
    // The diff vs live and the changed-on-server banner (E10-S07) read the object and change the
    // buffer only.
    CommandMeta::read(
        CommandId::EDITOR_ACCEPT_MERGE,
        "Accept Manifest Merge",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_CONVERT_TO_JSON,
        "Convert Manifest to JSON",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_CONVERT_TO_YAML,
        "Convert Manifest to YAML",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_KEEP_MINE,
        "Keep My Manifest Changes",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_MERGE_FROM_SERVER,
        "Merge Server Changes into Manifest",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_NEW_MANIFEST,
        "New Manifest",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::EDITOR_RELOAD_FROM_SERVER,
        "Reload Manifest from Server",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_SHOW_DIFF,
        "Show Diff vs Live",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    // Hover docs and completion (E10-S05) read the buffer and the cluster's schemas, and change
    // the buffer only when the user picks an entry.
    CommandMeta::read(
        CommandId::EDITOR_SHOW_HOVER,
        "Show Field Docs",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_TAKE_MINE,
        "Take My Side of a Merge Conflict",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_TAKE_SERVER,
        "Take the Server's Side of a Merge Conflict",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_TOGGLE_DIFF_LAYOUT,
        "Toggle Diff Side by Side / Inline",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    // "Read-only" here is the editor's view state, not the cluster's read-only guard.
    // Re-renders the text of an opened resource without `managedFields`; reads nothing more
    // from the cluster.
    CommandMeta::read(
        CommandId::EDITOR_TOGGLE_MANAGED_FIELDS,
        "Toggle Editor managedFields",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_TOGGLE_READ_ONLY,
        "Toggle Editor Read-Only",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    // Shows the Secret values the editor already holds in memory (or masks them again); the
    // revealed text is never written anywhere.
    CommandMeta::read(
        CommandId::EDITOR_TOGGLE_SECRET_MASK,
        "Toggle Editor Secret Values",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_TOGGLE_SOFT_WRAP,
        "Toggle Editor Soft Wrap",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_TRIGGER_COMPLETION,
        "Show Completions",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_USE_JSON,
        "Use JSON Mode in Editor",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    CommandMeta::read(
        CommandId::EDITOR_USE_YAML,
        "Use YAML Mode in Editor",
        CommandScope::Global,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    // The events view (E14-S03): it reads the app's `EventService` (one events feed per
    // cluster), never the cluster itself, and changes nothing in it. Copying and exporting
    // write the clipboard or a file the user picks; the messages are redacted on the way out.
    CommandMeta::read(
        CommandId::EVENTS_COPY_ROW,
        "Events: Copy Row",
        CommandScope::Cluster,
        NONE,
    )
    .in_views(EVENTS_VIEW),
    CommandMeta::read(
        CommandId::EVENTS_COPY_ROW_JSON,
        "Events: Copy Row as JSON",
        CommandScope::Cluster,
        NONE,
    )
    .in_views(EVENTS_VIEW),
    CommandMeta::read(
        CommandId::EVENTS_EXPORT_JSON,
        "Events: Export JSON",
        CommandScope::Cluster,
        NONE,
    )
    .in_views(EVENTS_VIEW),
    CommandMeta::read(
        CommandId::EVENTS_OPEN,
        "Show Events",
        CommandScope::Cluster,
        NONE,
    ),
    // Queues local context for the agent panel (E14-S08); the events are read from the
    // `EventService`'s buffer (its feed is started on demand). Changes nothing in the cluster.
    CommandMeta::read(
        CommandId::EVENTS_SEND_TO_AGENT,
        "Events: Send to Agent",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::EVENTS_TOGGLE_FAULTS,
        "Events: Toggle Faults Only",
        CommandScope::Cluster,
        NONE,
    )
    .in_views(EVENTS_VIEW),
    CommandMeta::read(
        CommandId::EVENTS_TOGGLE_PAUSE,
        "Events: Pause or Resume Live Tail",
        CommandScope::Cluster,
        NONE,
    )
    .in_views(EVENTS_VIEW),
    // Opens the user's `helm.path` setting; changes nothing in any cluster.
    CommandMeta::read(
        CommandId::HELM_LOCATE_BINARY,
        "Locate Helm Binary",
        CommandScope::Global,
        NONE,
    ),
    // Runs `helm version` to find the binary again; reads and changes nothing in any cluster.
    CommandMeta::read(
        CommandId::HELM_RECHECK,
        "Recheck Helm",
        CommandScope::Global,
        NONE,
    ),
    // Lists the bindings of the focused view; reads nothing from a cluster.
    CommandMeta::read(
        CommandId::HELP_SHOW,
        "Show Key Bindings",
        CommandScope::Global,
        NONE,
    ),
    // Patches the `ingressclass.kubernetes.io/is-default-class` annotation (a merge patch, with
    // a server dry run through the guard); low risk: it changes which class an Ingress without
    // one gets, and is undone by setting another class (or removing the annotation).
    CommandMeta::mutation(
        CommandId::INGRESS_CLASS_SET_DEFAULT,
        "Set Default Ingress Class",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::of("networking.k8s.io", "IngressClass")),
    CommandMeta::mutation(
        CommandId::JOB_RESUME,
        "Resume Job",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::OfKind {
        group: "batch",
        kind: "Job",
    }),
    CommandMeta::mutation(
        CommandId::JOB_SUSPEND,
        "Suspend Job",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::OfKind {
        group: "batch",
        kind: "Job",
    }),
    // The `:` jump bar's history only moves around the app: they read and change nothing in
    // a cluster, and run in read-only mode. What a typed line does is done by the navigation
    // commands it turns into (`resource::OpenList`, `namespace::Select`, `cluster::Select`, ...).
    CommandMeta::read(
        CommandId::JUMP_BACK,
        "Jump: Back in History",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::JUMP_FORWARD,
        "Jump: Forward in History",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::JUMP_LAST,
        "Jump: Previous View",
        CommandScope::Global,
        NONE,
    ),
    // Creates `keymap.json` in the user's config directory when it is missing (a commented
    // template, no secrets) and opens it in an editor; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::KEYMAP_OPEN_USER,
        "Open User Keymap",
        CommandScope::Global,
        NONE,
    ),
    // The kubeconfig commands change the user's settings list and Oxikube's own files, never a
    // cluster: not `mutating`, no guard tier. Removing a pasted kubeconfig deletes a file, so
    // the UI confirms it first and the handler refuses it for agents.
    CommandMeta::read(
        CommandId::KUBECONFIG_ADD_SOURCE,
        "Add Kubeconfig Source",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::KUBECONFIG_RELOAD,
        "Reload Kubeconfigs",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::KUBECONFIG_REMOVE_SOURCE,
        "Remove Kubeconfig Source",
        CommandScope::Global,
        NONE,
    ),
    // The log view's commands (E08-S02, search E08-S03): they change what a view shows or which
    // stream it reads, never the cluster. Reopening a stream reads logs, so those need the logs
    // capability. Kept sorted by id (the table is binary-searched). The local actions on what a
    // view holds (E08-S06) clear the buffer, never the cluster's logs; a save writes only the
    // file the user picks, so none of them is a mutation. Reconnecting and following a
    // replacement pod (E08-S07) read logs (and the pod's owner), nothing else.
    CommandMeta::read(
        CommandId::LOGS_CLEAR,
        "Logs: Clear",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_CLOSE_SEARCH,
        "Logs: Close Search",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_COLLAPSE_LINE,
        "Logs: Collapse Line",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_COPY,
        "Logs: Copy Lines",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_FIND,
        "Logs: Find",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_FOLLOW_REPLACEMENT,
        "Logs: Follow Replacement Pod",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_MARK,
        "Logs: Mark Line",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_NEXT_MATCH,
        "Logs: Next Match",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_PREVIOUS_MATCH,
        "Logs: Previous Match",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_RECONNECT,
        "Logs: Reconnect",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_SAVE,
        "Logs: Save to File",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_SELECT_CONTAINER,
        "Logs: Select Container",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    // Queues local context for the agent panel; the lines are read from the view's own buffer.
    CommandMeta::read(
        CommandId::LOGS_SEND_TO_AGENT,
        "Logs: Send to Agent",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_SET_RANGE,
        "Logs: Set Range",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    // Starts kubectl on this machine in a terminal tab, with the cluster's kubeconfig in its
    // environment; `kubectl logs` only reads. It is an interactive action: it has a tool stub like
    // every command, but no agent runs processes on the user's machine until the agent phase
    // decides how (the guard's initiator rules are the place for that).
    CommandMeta::read(
        CommandId::LOGS_TAIL_IN_TERMINAL,
        "Logs: Tail in Terminal (kubectl)",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_AUTOSCROLL,
        "Logs: Toggle Autoscroll",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_CASE,
        "Logs: Toggle Case Sensitivity",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_FILTER_MODE,
        "Logs: Toggle Filter Mode",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_FULLSCREEN,
        "Logs: Toggle Fullscreen",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_INVERSE,
        "Logs: Toggle Inverse Match",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_JSON_MODE,
        "Logs: Toggle JSON Mode",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_LEVEL,
        "Logs: Toggle Level",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_LINE,
        "Logs: Expand or Collapse Line",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_PREVIOUS,
        "Logs: Toggle Previous Container",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_SOURCE,
        "Logs: Toggle Source",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_TIMESTAMPS,
        "Logs: Toggle Timestamps",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::read(
        CommandId::LOGS_TOGGLE_WRAP,
        "Logs: Toggle Wrap",
        CommandScope::Selection,
        NONE,
    )
    .in_views(LOG_VIEW),
    CommandMeta::mutation(
        CommandId::NAMESPACE_CREATE,
        "Create Namespace",
        CommandScope::Cluster,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS),
    CommandMeta::mutation(
        CommandId::NAMESPACE_DELETE,
        "Delete Namespace",
        CommandScope::Selection,
        Risk::Irreversible,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Namespace")),
    CommandMeta::read(
        CommandId::NAMESPACE_SELECT,
        "Select Namespaces",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NAMESPACE_TOGGLE_FAVOURITE,
        "Toggle Favourite Namespace",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NAMESPACE_USE_FAVOURITE,
        "Use Favourite Namespace",
        CommandScope::Cluster,
        NONE,
    )
    .in_views(&[ViewContext::Table]),
    CommandMeta::mutation(
        CommandId::NODE_CORDON,
        "Cordon Node",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Node")),
    CommandMeta::mutation(
        CommandId::NODE_DRAIN,
        "Drain Node",
        CommandScope::Selection,
        Risk::High,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Node")),
    // A shell on a node is root on it: the command creates a privileged pod (a mutation, blocked
    // on a read-only cluster, a confirmation that names the node and the image) and opens a
    // session in it; its tool stub is unsafe, interactive and hidden from agents.
    CommandMeta::interactive_mutation(
        CommandId::NODE_SHELL,
        "Node Shell",
        CommandScope::Selection,
        Risk::Medium,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Node")),
    CommandMeta::mutation(
        CommandId::NODE_UNCORDON,
        "Uncordon Node",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Node")),
    // The notification centre (E14-S05): reading it, marking it read and muting a cluster's
    // notifications touch only this app's own list and its local state db, never a cluster, so
    // none is a mutation and all run on a read-only cluster.
    CommandMeta::read(
        CommandId::NOTIFICATIONS_MARK_ALL_READ,
        "Mark All Notifications Read",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NOTIFICATIONS_MARK_READ,
        "Mark Notification Read",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NOTIFICATIONS_MUTE_CLUSTER,
        "Mute Cluster Notifications",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NOTIFICATIONS_OPEN_LINK,
        "Open Notification Link",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NOTIFICATIONS_TOGGLE_PANEL,
        "Toggle Notification Centre",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::NOTIFICATIONS_UNMUTE_CLUSTER,
        "Unmute Cluster Notifications",
        CommandScope::Cluster,
        NONE,
    ),
    // Forgets the palette's recent commands (a list of ids on this machine); touches no cluster.
    CommandMeta::read(
        CommandId::PALETTE_CLEAR_RECENTS,
        "Clear Recent Commands",
        CommandScope::Global,
        NONE,
    ),
    // Opens the `:` bar; what is typed in it runs other commands, each with its own guard tier.
    CommandMeta::read(
        CommandId::PALETTE_OPEN_JUMP,
        "Open Jump Bar",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PALETTE_TOGGLE,
        "Toggle Command Palette",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PALETTE_TOGGLE_SHOW_ALL,
        "Show Unavailable Commands",
        CommandScope::Global,
        NONE,
    )
    .in_views(&[ViewContext::Palette]),
    // The exec class (`CommandMeta::exec`): not mutations, no confirmation, blocked on a
    // read-only cluster unless `exec_in_read_only` allows it, audited on every open.
    CommandMeta::exec(CommandId::POD_ATTACH, "Attach", CommandScope::Selection)
        .in_views(TABLE_VIEWS)
        .selecting(SelectionKind::core("Pod")),
    // Adds an ephemeral container to the pod (`patch` on `pods/ephemeralcontainers`), which cannot
    // be removed or edited until the pod is deleted: a mutation, so read-only blocks it, a simple
    // confirmation names the pod, image and target (low risk, E09-S10), and it is audited. It is
    // not exec-class: the mutation pipeline applies. Its tool stub is interactive and hidden from
    // agents by default.
    CommandMeta::interactive_mutation(
        CommandId::POD_DEBUG,
        "Debug",
        CommandScope::Selection,
        Risk::Low,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    CommandMeta::mutation(
        CommandId::POD_DELETE,
        "Delete Pod",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    // E12-S03. Evicting or deleting one pod is routine (medium: a simple confirm naming the pod
    // and cluster); a force delete skips the kubelet's grace and so takes the typed name (high).
    // A sweep of Completed and Failed pods is one delete each at medium.
    CommandMeta::mutation(
        CommandId::POD_EVICT,
        "Evict Pod",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    CommandMeta::exec(
        CommandId::POD_EXEC,
        "Exec into Container",
        CommandScope::Selection,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    CommandMeta::mutation(
        CommandId::POD_FORCE_DELETE,
        "Force Delete Pod",
        CommandScope::Selection,
        Risk::High,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    // Navigation: reads the pod and opens another object's detail.
    CommandMeta::read(
        CommandId::POD_JUMP_TO_OWNER,
        "Jump to Owner",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    CommandMeta::read(
        CommandId::POD_PORT_FORWARD,
        "Port-Forward",
        CommandScope::Selection,
        Capabilities::PORTFORWARD,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    CommandMeta::mutation(
        CommandId::POD_SANITIZE,
        "Sanitize Pods",
        CommandScope::Cluster,
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS),
    CommandMeta::exec(CommandId::POD_SHELL, "Shell", CommandScope::Selection)
        .in_views(TABLE_VIEWS)
        .selecting(SelectionKind::core("Pod")),
    CommandMeta::read(
        CommandId::POD_SHOW_NODE,
        "Show Node",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    CommandMeta::read(
        CommandId::POD_VIEW_LOGS,
        "View Logs",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Pod")),
    // The port-forward manager (E15-S03). A forward changes nothing in the cluster, so these are
    // reads; the manager refuses a start on a read-only cluster unless
    // `portforward.allowed_in_readonly`. Stop, restart and remove act on the manager's own list.
    // Favourites (E15-S06) are local state, not cluster state. Rows stay sorted by id.
    CommandMeta::read(
        CommandId::PORTFORWARD_COPY_URL,
        "Copy Port-Forward URL",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PORTFORWARD_FAVOURITE_ADD,
        "Add Port-Forward Favourite",
        CommandScope::Selection,
        Capabilities::PORTFORWARD,
    )
    .selecting(SelectionKind::AnyOf(FORWARDABLE_KINDS)),
    CommandMeta::read(
        CommandId::PORTFORWARD_FAVOURITE_REMOVE,
        "Remove Port-Forward Favourite",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PORTFORWARD_FAVOURITE_TOGGLE_AUTO_START,
        "Toggle Port-Forward Favourite Auto-Start",
        CommandScope::Global,
        NONE,
    ),
    // The manager panel (E15-S05): local, UI-side actions on the manager's own list.
    CommandMeta::read(
        CommandId::PORTFORWARD_OPEN_IN_BROWSER,
        "Open Port-Forward in Browser",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PORTFORWARD_REMOVE,
        "Remove Port-Forward",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PORTFORWARD_RESTART,
        "Restart Port-Forward",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PORTFORWARD_START,
        "Start Port-Forward",
        CommandScope::Selection,
        Capabilities::PORTFORWARD,
    )
    .selecting(SelectionKind::AnyOf(FORWARDABLE_KINDS)),
    CommandMeta::read(
        CommandId::PORTFORWARD_STOP,
        "Stop Port-Forward",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::PORTFORWARD_TOGGLE_PANEL,
        "Toggle Port Forwards",
        CommandScope::Global,
        NONE,
    ),
    // The resize dialog (E12-S15) is the confirmation's front: it checks growth and
    // `allowVolumeExpansion`, names the old and the new size, then the guard asks (simple confirm
    // naming the claim and the cluster). The handler dry-runs the patch first.
    CommandMeta::mutation(
        CommandId::PVC_RESIZE,
        "Resize Volume Claim",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    )
    // Scoped to claims in the table only: the dialog is opened by the table surface, which the
    // detail drawer does not have, and the palette must not offer it on other kinds.
    .in_views(TABLE_VIEW)
    .selecting(SelectionKind::core("PersistentVolumeClaim")),
    // The manifest editor runs it through its own flow (E10-S08): a server dry run, the diff
    // preview (which is the confirmation, hence `Low`), then the apply. Agents call it directly;
    // the guard and the handler's own dry run apply to them alike.
    CommandMeta::mutation(
        CommandId::RESOURCE_APPLY,
        "Apply Manifest",
        CommandScope::Cluster,
        Risk::Low,
        NONE,
    )
    .in_views(EDITOR_VIEW),
    // Writes the user's clipboard, never the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_COPY_LABEL,
        "Copy Label",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Writes the user's clipboard, never the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_COPY_NAME,
        "Copy Resource Name",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::Many),
    // Writes the user's clipboard, never the cluster: the text is what the YAML tab shows.
    CommandMeta::read(
        CommandId::RESOURCE_COPY_YAML,
        "Copy YAML",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Opens a manifest editor on a template (E10-S09); like `editor::NewManifest` it changes
    // nothing in a cluster. The objects are made by `resource::Apply`, one per document of the
    // buffer, each through the guard (read-only blocks it, the dry-run preview confirms it, every
    // document is audited).
    CommandMeta::read(
        CommandId::RESOURCE_CREATE,
        "Create Resource…",
        CommandScope::Cluster,
        NONE,
    ),
    CommandMeta::mutation(
        CommandId::RESOURCE_DELETE,
        "Delete Resource",
        CommandScope::Selection,
        // The floor: the guard raises it by target (Namespace, Node, PersistentVolume, a cascading
        // delete: `Command::effective_risk`), so an ordinary object takes a simple confirm.
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::Many),
    // Opens the manifest in the editor; the change reaches the cluster only through
    // `resource::Apply`, which is guarded.
    CommandMeta::read(
        CommandId::RESOURCE_EDIT,
        "Edit Resource",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Find in the text a detail shows (YAML, Describe): a view-local search, changes nothing.
    CommandMeta::read(
        CommandId::RESOURCE_FIND,
        "Find in Resource Text",
        CommandScope::Selection,
        NONE,
    )
    .in_views(DETAIL_VIEW)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_NEXT_MATCH,
        "Next Match in Resource Text",
        CommandScope::Selection,
        NONE,
    )
    .in_views(DETAIL_VIEW)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_OPEN,
        "Open Resource",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_OPEN_LIST,
        "Open Resource List",
        CommandScope::ResourceKind,
        NONE,
    ),
    // Moves a view between a drawer and a tab; changes nothing in the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_PIN_DETAIL,
        "Pin Detail as Tab",
        CommandScope::Selection,
        NONE,
    )
    .in_views(DETAIL_VIEW)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_PREVIOUS_MATCH,
        "Previous Match in Resource Text",
        CommandScope::Selection,
        NONE,
    )
    .in_views(DETAIL_VIEW)
    .selecting(SelectionKind::One),
    // Reads the object and its events again (`kubectl describe`); changes nothing.
    CommandMeta::read(
        CommandId::RESOURCE_REFRESH_DESCRIBE,
        "Refresh Describe",
        CommandScope::Selection,
        NONE,
    )
    .in_views(DETAIL_VIEW)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_RETRY_FEED,
        "Retry Resource Feed",
        CommandScope::ResourceKind,
        NONE,
    )
    .in_views(TABLE_VIEW),
    // Writes a local file the user picks, never the cluster: the text is what the YAML tab shows.
    CommandMeta::read(
        CommandId::RESOURCE_SAVE_YAML,
        "Save YAML as File",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_SELECT_ALL,
        "Select All Resources",
        CommandScope::ResourceKind,
        NONE,
    )
    .in_views(TABLE_VIEW),
    // Opens an object's "Used by" tab (E12-S18): the workloads whose Pods name it. Reads the Pods
    // the session already watches and, for the owner chain, a few objects; navigation, a read.
    // `One`: the kinds it applies to are the row action's filter, because a command can name a
    // single kind only.
    CommandMeta::read(
        CommandId::RESOURCE_SHOW_USED_BY,
        "Show Used By",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // A display option of the open YAML tab; changes nothing in the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_TOGGLE_MANAGED_FIELDS,
        "Toggle Managed Fields",
        CommandScope::Selection,
        NONE,
    )
    .in_views(DETAIL_VIEW)
    .selecting(SelectionKind::One),
    // Shows the describe tab of the detail; reads the object and its events, changes nothing.
    CommandMeta::read(
        CommandId::RESOURCE_VIEW_DESCRIBE,
        "Describe",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Shows the events tab of the detail (E14-S04); reads the app's events index, changes
    // nothing in the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_VIEW_EVENTS,
        "View Events",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Shows the metrics tab of the detail; reads Prometheus through the cluster's own service
    // proxy or the configured endpoint, changes nothing in the cluster.
    CommandMeta::read(
        CommandId::RESOURCE_VIEW_METRICS,
        "View Metrics",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::RESOURCE_VIEW_YAML,
        "View YAML",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Reads the object and shows it in a read-only editor tab; allowed in read-only mode.
    CommandMeta::read(
        CommandId::RESOURCE_VIEW_YAML_IN_EDITOR,
        "View YAML in Editor",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Opens a dialog that builds a Secret manifest in an editor tab (E12-S11); like
    // `resource::Create` it changes nothing in a cluster. The object is made by `resource::Apply`
    // (risk low, the dry-run diff with the values masked is the confirmation, blocked read-only).
    CommandMeta::read(
        CommandId::SECRET_CREATE,
        "Create Secret…",
        CommandScope::Cluster,
        NONE,
    ),
    // Opens the Secret in the manifest editor with `data` shown as plaintext (E12-S11); like
    // `resource::Edit` the change reaches the cluster only through `resource::Apply`, which is
    // guarded (risk low, blocked read-only, the values re-encoded to base64 first, audited
    // without them).
    CommandMeta::read(
        CommandId::SECRET_EDIT_DECODED,
        "Edit Secret Decoded",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Secret")),
    // Reads the Secret once (a `get`) and shows its values in memory (E12-S11); changes nothing in
    // the cluster and writes nothing to disk. Allowed in read-only mode.
    CommandMeta::read(
        CommandId::SECRET_REVEAL,
        "Reveal Secret Values",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Secret")),
    // The bulk actions of a selection (E12-S19). Each is one guarded command over all the objects:
    // the guard asks once, at the tier of the riskiest object (`Command::effective_risk` takes the
    // maximum over the targets), refuses the whole batch on a read-only cluster before anything is
    // sent, and the handler audits every object on its own under a shared batch id.
    //
    // The declared risk is the floor: a delete is medium (a simple confirm) until an object in the
    // selection raises it (a Namespace, a cascading delete: type the name).
    CommandMeta::mutation(
        CommandId::SELECTION_DELETE,
        "Delete Selection",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::Many),
    // Labels can orphan pods from their Service or controller, so a label edit is a medium risk
    // (one confirm, with the warning); annotations ride the same command.
    CommandMeta::mutation(
        CommandId::SELECTION_LABEL,
        "Edit Labels of Selection",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::Many),
    // Only when every selected object is a Deployment, StatefulSet or DaemonSet.
    CommandMeta::mutation(
        CommandId::SELECTION_RESTART,
        "Restart Selection",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::AnyOf(ROLLABLE_KINDS)),
    // Starts a port-forward to a pod behind the Service and opens the local end in the browser. A
    // forward changes nothing in the cluster, so it is a read like `pod::PortForward` (what the
    // forward may do is the port-forward manager's guard policy, E15); it needs the capability.
    CommandMeta::read(
        CommandId::SERVICE_OPEN_IN_BROWSER,
        "Open Service in Browser",
        CommandScope::Selection,
        Capabilities::PORTFORWARD,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Service")),
    // Opens a table of pods filtered by the Service's selector (`-l`): navigation, a read.
    CommandMeta::read(
        CommandId::SERVICE_SHOW_PODS,
        "Show Service Pods",
        CommandScope::Selection,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::core("Service")),
    // One annotation on one StorageClass (and the previous default's, when asked): reversible by
    // setting another class, hence `Low`; the per-session skip of low-risk confirms is E19's.
    CommandMeta::mutation(
        CommandId::STORAGECLASS_SET_DEFAULT,
        "Set Default Storage Class",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    // Table only and storage classes only, for the same reason as `pvc::Resize`: the dialog
    // (current default, clear or keep it) is the table surface's.
    .in_views(TABLE_VIEW)
    .selecting(SelectionKind::OfKind {
        group: "storage.k8s.io",
        kind: "StorageClass",
    }),
    // Moves focus inside a window, never touches the cluster: allowed in read-only mode.
    CommandMeta::read(
        CommandId::TABLE_FOCUS_FILTER,
        "Focus Table Filter",
        CommandScope::ResourceKind,
        NONE,
    )
    .in_views(TABLE_VIEW),
    // Changes what one view shows on this machine, never the cluster: allowed in read-only mode.
    CommandMeta::read(
        CommandId::TABLE_SET_FILTER,
        "Set Table Filter",
        CommandScope::ResourceKind,
        NONE,
    )
    .in_views(TABLE_VIEW),
    // A display option of the open tables of a kind; changes nothing in the cluster.
    CommandMeta::read(
        CommandId::TABLE_TOGGLE_WIDE,
        "Toggle Wide Columns",
        CommandScope::ResourceKind,
        NONE,
    )
    .in_views(TABLE_VIEW),
    // Drops the scrollback this machine holds for the user's own session; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_CLEAR,
        "Clear Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Ends a shell on this machine the user opened; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_CLOSE,
        "Close Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Copies text the user sees to the clipboard; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_COPY,
        "Copy Terminal Selection",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Starts the user's own shell on this machine (with the cluster's kubeconfig in its
    // environment); the cluster is only touched by what the user then types, outside the guard.
    CommandMeta::read(
        CommandId::TERMINAL_NEW,
        "New Terminal",
        CommandScope::Global,
        NONE,
    ),
    // Opens the user's browser or file opener; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_OPEN_LINK,
        "Open Terminal Link",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Types the clipboard into the user's own session (a multi-line paste asks first); the
    // cluster is only touched by what the user's shell then does, outside the guard's reach.
    CommandMeta::read(
        CommandId::TERMINAL_PASTE,
        "Paste into Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Opens the pod session again through the exec service, which re-checks read-only mode and
    // the exec capability itself; the command only asks the terminal to start over.
    CommandMeta::read(
        CommandId::TERMINAL_RECONNECT,
        "Reconnect Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Starts the user's own shell on this machine again; never touches a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_RESTART,
        "Restart Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Moves the view of a terminal's own history; never touches a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SCROLL_LINE_DOWN,
        "Scroll Terminal Line Down",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Moves the view of a terminal's own history; never touches a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SCROLL_LINE_UP,
        "Scroll Terminal Line Up",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Moves the view of a terminal's own history; never touches a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SCROLL_PAGE_DOWN,
        "Scroll Terminal Page Down",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Moves the view of a terminal's own history; never touches a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SCROLL_PAGE_UP,
        "Scroll Terminal Page Up",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Searches the text a terminal shows, on this machine; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SEARCH,
        "Search Terminal Scrollback",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Searches the text a terminal shows, on this machine; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SEARCH_CLOSE,
        "Close Terminal Search",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Searches the text a terminal shows, on this machine; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SEARCH_NEXT,
        "Next Terminal Search Match",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Searches the text a terminal shows, on this machine; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SEARCH_PREVIOUS,
        "Previous Terminal Search Match",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Selects text a terminal shows; never reads or changes a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SELECT_ALL,
        "Select All in Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    // Starts another shell on this machine beside the active pane; never touches a cluster.
    CommandMeta::read(
        CommandId::TERMINAL_SPLIT,
        "Split Terminal",
        CommandScope::Global,
        NONE,
    )
    .in_views(TERMINAL_VIEW),
    CommandMeta::read(
        CommandId::VIEW_OPEN,
        "Open View",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_ZOOM_IN,
        "Zoom In",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_ZOOM_OUT,
        "Zoom Out",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::VIEW_ZOOM_RESET,
        "Reset Zoom",
        CommandScope::Global,
        NONE,
    ),
    CommandMeta::read(
        CommandId::WINDOW_NEW,
        "New Window",
        CommandScope::Global,
        NONE,
    ),
    // A rolling restart replaces pods one batch at a time and changes no spec: low risk (E12-S05).
    CommandMeta::mutation(
        CommandId::WORKLOAD_RESTART,
        "Restart Workload",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // E12-S06. Medium: it replaces the pod template, so every pod of the workload is replaced by
    // a rolling update, but the revision it leaves is still in the history to go forward again.
    CommandMeta::mutation(
        CommandId::WORKLOAD_ROLLBACK,
        "Rollback Workload",
        CommandScope::Selection,
        Risk::Medium,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Low for N > 0, raised to Medium for N = 0 by `Command::effective_risk` (E12-S04).
    CommandMeta::mutation(
        CommandId::WORKLOAD_SCALE,
        "Scale Workload",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    // Changes the pod template, so it rolls the workload: low risk, one confirmation (E12-S05).
    CommandMeta::mutation(
        CommandId::WORKLOAD_SET_IMAGE,
        "Set Workload Image",
        CommandScope::Selection,
        Risk::Low,
        NONE,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
    CommandMeta::read(
        CommandId::WORKLOAD_VIEW_LOGS,
        "View Logs",
        CommandScope::Selection,
        Capabilities::LOGS,
    )
    .in_views(TABLE_VIEWS)
    .selecting(SelectionKind::One),
];

/// Find a command's metadata by id (binary search, no allocation).
pub fn lookup(id: CommandId) -> Option<&'static CommandMeta> {
    lookup_str(id.as_str())
}

/// Find a command's metadata by its `namespace::Verb` string.
pub fn lookup_str(id: &str) -> Option<&'static CommandMeta> {
    COMMANDS
        .binary_search_by(|meta| meta.id.as_str().cmp(id))
        .ok()
        .map(|i| &COMMANDS[i])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::safety::{ConfirmTier, Initiator};

    /// Mutating commands that deliberately run without a confirmation. Empty on
    /// purpose: adding an entry here is a reviewed decision that a mutation is
    /// safe to run unconfirmed (say why in a comment next to the entry).
    const UNCONFIRMED_MUTATING: &[&str] = &[];

    #[test]
    fn ids_are_well_formed_unique_and_sorted() {
        let mut seen = HashSet::new();
        for pair in COMMANDS.windows(2) {
            assert!(
                pair[0].id < pair[1].id,
                "registry must be sorted by id: {} before {}",
                pair[0].id,
                pair[1].id
            );
        }
        for meta in COMMANDS {
            assert!(
                crate::command::is_well_formed(meta.id.as_str()),
                "{}",
                meta.id
            );
            assert!(seen.insert(meta.id), "duplicate id {}", meta.id);
            assert!(!meta.title.trim().is_empty(), "{} has no title", meta.id);
        }
    }

    #[test]
    fn tool_names_are_unique() {
        let mut seen = HashSet::new();
        for meta in COMMANDS {
            let tool = meta.id.tool_name();
            assert!(
                tool.starts_with("k8s.") || tool.starts_with("app.") || tool.starts_with("helm."),
                "{tool}"
            );
            assert!(seen.insert(tool.clone()), "duplicate tool name {tool}");
        }
    }

    #[test]
    fn mutating_commands_are_guarded() {
        for meta in COMMANDS {
            if meta.mutating {
                assert!(
                    meta.needs.contains(Capabilities::MUTATE),
                    "{} mutates but does not need MUTATE",
                    meta.id
                );
                assert!(!meta.privileged, "{} is mutating and privileged", meta.id);
                let risk = meta.risk.expect("mutating command declares a risk");
                assert_eq!(meta.confirm, risk.confirm_tier(), "{}", meta.id);
                if meta.confirm == ConfirmTier::None {
                    assert!(
                        UNCONFIRMED_MUTATING.contains(&meta.id.as_str()),
                        "{} mutates with no confirmation and is not on the allow list",
                        meta.id
                    );
                }
            } else {
                assert_eq!(meta.confirm, ConfirmTier::None, "{}", meta.id);
                assert_eq!(meta.risk, None, "{}", meta.id);
                assert!(!meta.needs.contains(Capabilities::MUTATE), "{}", meta.id);
            }
        }
        for id in UNCONFIRMED_MUTATING {
            let meta = lookup_str(id).expect("allow-list entry is a registered id");
            assert!(meta.mutating, "{id} is on the allow list but not mutating");
        }
    }

    #[test]
    fn read_only_toggle_is_privileged_and_refused_for_agents() {
        let meta = lookup(CommandId::CLUSTER_TOGGLE_READ_ONLY).unwrap();
        assert!(meta.privileged);
        assert!(!meta.mutating, "must stay runnable on a read-only cluster");
        assert!(meta.allows(Initiator::Ui));
        assert!(meta.allows(Initiator::Command));
        assert!(!meta.allows(Initiator::Agent));
        assert!(!meta.allows(Initiator::Plugin));
        let privileged: Vec<_> = COMMANDS.iter().filter(|m| m.privileged).collect();
        assert_eq!(privileged.len(), 1, "privileged is a reviewed allow-list");
        let delete = lookup(CommandId::POD_DELETE).unwrap();
        assert!(delete.allows(Initiator::Agent));
    }

    #[test]
    fn connecting_is_a_read_command_with_a_tool_name() {
        // Connect and disconnect read from the cluster and change nothing in it (E06-S03): they
        // stay runnable on a read-only cluster and need no confirmation.
        for (id, tool) in [
            (CommandId::CLUSTER_CONNECT, "app.cluster_connect"),
            (CommandId::CLUSTER_DISCONNECT, "app.cluster_disconnect"),
            (CommandId::CLUSTER_RECONNECT, "app.cluster_reconnect"),
            (
                CommandId::CLUSTER_CANCEL_CONNECT,
                "app.cluster_cancel_connect",
            ),
            (
                CommandId::CLUSTER_TOGGLE_FAVOURITE,
                "app.cluster_toggle_favourite",
            ),
        ] {
            let meta = lookup(id).expect("registered");
            assert!(!meta.mutating && !meta.privileged, "{id}");
            assert_eq!(meta.confirm, ConfirmTier::None, "{id}");
            assert_eq!(id.tool_name(), tool);
        }
    }

    #[test]
    fn lookup_finds_every_entry() {
        for meta in COMMANDS {
            assert_eq!(lookup(meta.id), Some(meta));
        }
        assert_eq!(lookup_str("pod::Explode"), None);
        assert_eq!(lookup_str(""), None);
    }

    #[test]
    fn spot_check_declared_tiers() {
        let get = |id: CommandId| lookup(id).unwrap();
        assert_eq!(get(CommandId::RESOURCE_DELETE).confirm, ConfirmTier::Simple);
        assert_eq!(get(CommandId::NODE_DRAIN).confirm, ConfirmTier::TypeName);
        assert_eq!(get(CommandId::WORKLOAD_SCALE).confirm, ConfirmTier::Simple);
        // A rollback replaces the pod template: medium risk, one simple confirmation (E12-S06).
        let rollback = get(CommandId::WORKLOAD_ROLLBACK);
        assert!(rollback.mutating);
        assert_eq!(rollback.risk, Some(Risk::Medium));
        assert_eq!(rollback.confirm, ConfirmTier::Simple);
        assert_eq!(rollback.id.tool_name(), "k8s.workload_rollback");
        assert_eq!(get(CommandId::POD_VIEW_LOGS).needs, Capabilities::LOGS);
        assert!(!get(CommandId::POD_PORT_FORWARD).mutating);
        // A debug container is a low-risk mutation with a simple confirm that ends in a terminal.
        let debug = get(CommandId::POD_DEBUG);
        assert!(debug.mutating && debug.interactive && !debug.exec);
        assert_eq!(debug.risk, Some(Risk::Low));
        assert_eq!(debug.confirm, ConfirmTier::Simple);
        assert!(
            debug
                .needs
                .contains(Capabilities::EXEC | Capabilities::MUTATE)
        );
        assert_eq!(CommandId::POD_DEBUG.tool_name(), "k8s.pod_debug");
    }

    #[test]
    fn the_secret_commands_open_views_and_write_only_through_resource_apply() {
        // E12-S11: revealing reads the Secret into memory, "edit decoded" and "create" open an
        // editor; the object is written by `resource::Apply` (risk low), so none of the three is
        // a mutation itself, and each has its MCP tool stub.
        for (id, tool) in [
            (CommandId::SECRET_REVEAL, "k8s.secret_reveal"),
            (CommandId::SECRET_EDIT_DECODED, "k8s.secret_edit_decoded"),
            (CommandId::SECRET_CREATE, "k8s.secret_create"),
        ] {
            let meta = lookup(id).expect("declared");
            assert!(!meta.mutating && !meta.privileged && !meta.exec, "{id}");
            assert_eq!(meta.confirm, ConfirmTier::None, "{id}");
            assert_eq!(id.tool_name(), tool);
        }
        let apply = lookup(CommandId::RESOURCE_APPLY).expect("declared");
        assert_eq!(apply.risk, Some(Risk::Low));
        assert_eq!(apply.confirm, ConfirmTier::Simple);
        // Reveal and edit act on a selected Secret only.
        for id in [CommandId::SECRET_REVEAL, CommandId::SECRET_EDIT_DECODED] {
            let meta = lookup(id).expect("declared");
            assert!(meta.availability.selection.needs_selection(), "{id}");
        }
    }

    #[test]
    fn restart_and_set_image_are_low_risk_one_confirmation_mutations_with_tool_stubs() {
        // E12-S05: both change a workload's pod template and roll it; the guard confirms once.
        for (id, title, tool) in [
            (
                CommandId::WORKLOAD_RESTART,
                "Restart Workload",
                "k8s.workload_restart",
            ),
            (
                CommandId::WORKLOAD_SET_IMAGE,
                "Set Workload Image",
                "k8s.workload_set_image",
            ),
        ] {
            let meta = lookup(id).expect("registered");
            assert!(meta.mutating && !meta.exec && !meta.privileged, "{id}");
            assert_eq!(meta.title, title);
            assert_eq!(meta.risk, Some(Risk::Low), "{id}");
            assert_eq!(meta.tool_risk(), Some(Risk::Low), "{id}");
            assert_eq!(meta.confirm, ConfirmTier::Simple, "{id}");
            assert_eq!(id.tool_name(), tool);
            assert!(meta.availability.selection.needs_selection(), "{id}");
        }
    }

    #[test]
    fn view_events_is_a_read_with_a_tool_stub() {
        // E14-S04: the warning badge of a row opens the Events tab, which reads the app's
        // events index and changes nothing, so no guard tier and no confirmation.
        let id = CommandId::RESOURCE_VIEW_EVENTS;
        let meta = lookup(id).expect("declared");
        assert!(!meta.mutating && !meta.privileged && !meta.exec);
        assert_eq!(meta.confirm, ConfirmTier::None);
        assert!(!meta.availability.requires_writable);
        assert!(meta.availability.selection.needs_selection());
        assert_eq!(id.tool_name(), "k8s.resource_view_events");
    }

    #[test]
    fn show_used_by_is_a_read_with_a_tool_stub() {
        // E12-S18: `u` opens the "Used by" tab; it reads the Pods the session already watches and
        // changes nothing, so no guard tier and no confirmation, allowed on a read-only cluster.
        let id = CommandId::RESOURCE_SHOW_USED_BY;
        let meta = lookup(id).expect("declared");
        assert!(!meta.mutating && !meta.privileged && !meta.exec);
        assert_eq!(meta.confirm, ConfirmTier::None);
        assert_eq!(meta.risk, None);
        assert!(!meta.availability.requires_writable);
        assert!(meta.availability.selection.needs_selection());
        assert_eq!(id.tool_name(), "k8s.resource_show_used_by");
    }

    #[test]
    fn exec_commands_are_their_own_class() {
        let exec: Vec<_> = COMMANDS.iter().filter(|m| m.exec).map(|m| m.id).collect();
        assert_eq!(
            exec,
            [
                CommandId::POD_ATTACH,
                CommandId::POD_EXEC,
                CommandId::POD_SHELL
            ],
            "the exec class is a reviewed allow-list"
        );
        for meta in COMMANDS.iter().filter(|m| m.exec) {
            // Not a mutation (no object changes, no confirmation) and not privileged (an agent
            // may ask; the tool stub is hidden from agents by default instead).
            assert!(!meta.mutating && !meta.privileged, "{}", meta.id);
            assert_eq!(meta.confirm, ConfirmTier::None, "{}", meta.id);
            assert_eq!(meta.risk, None, "{}", meta.id);
            assert_eq!(meta.needs, Capabilities::EXEC, "{}", meta.id);
            assert_eq!(meta.tool_risk(), Some(Risk::High), "{}", meta.id);
            assert!(meta.id.tool_name().starts_with("k8s.pod_"), "{}", meta.id);
        }
        assert_eq!(CommandId::POD_EXEC.tool_name(), "k8s.pod_exec");
        assert!(
            COMMANDS
                .iter()
                .filter(|m| !m.exec)
                .all(|m| m.tool_risk() == m.risk
                    || m.id == CommandId::RESOURCE_DELETE
                    || m.id == CommandId::SELECTION_DELETE
                    || m.id == CommandId::WORKLOAD_SCALE)
        );
    }

    #[test]
    fn the_bulk_commands_are_guarded_selection_mutations() {
        for (id, title, tool, risk) in [
            (
                CommandId::SELECTION_DELETE,
                "Delete Selection",
                "k8s.selection_delete",
                Risk::Medium,
            ),
            (
                CommandId::SELECTION_LABEL,
                "Edit Labels of Selection",
                "k8s.selection_label",
                Risk::Medium,
            ),
            (
                CommandId::SELECTION_RESTART,
                "Restart Selection",
                "k8s.selection_restart",
                Risk::Low,
            ),
        ] {
            let meta = lookup(id).expect("registered");
            assert!(meta.mutating && !meta.exec && !meta.privileged, "{id}");
            assert_eq!(meta.title, title);
            assert_eq!(meta.risk, Some(risk), "{id}");
            assert_eq!(meta.confirm, ConfirmTier::Simple, "{id}");
            assert_eq!(id.tool_name(), tool);
            assert!(meta.availability.requires_writable, "{id}");
            assert!(meta.availability.selection.needs_selection(), "{id}");
        }
        // Only a restart is picky about the kinds in the selection.
        assert!(matches!(
            lookup(CommandId::SELECTION_RESTART)
                .unwrap()
                .availability
                .selection,
            SelectionKind::AnyOf(kinds) if kinds.len() == 3
        ));
        assert_eq!(
            lookup(CommandId::SELECTION_DELETE)
                .unwrap()
                .availability
                .selection,
            SelectionKind::Many
        );
    }

    #[test]
    fn node_shell_is_a_guarded_interactive_mutation() {
        let meta = lookup(CommandId::NODE_SHELL).expect("declared");
        // Not the exec class (that one is never confirmed and may run on a read-only cluster):
        // it creates a privileged pod.
        assert!(meta.mutating && !meta.exec && meta.interactive && !meta.privileged);
        assert_eq!(meta.risk, Some(Risk::Medium));
        assert_eq!(meta.confirm, ConfirmTier::Simple);
        assert!(
            meta.needs
                .contains(Capabilities::MUTATE | Capabilities::EXEC)
        );
        assert_eq!(meta.tool_risk(), Some(Risk::Medium));
        assert_eq!(CommandId::NODE_SHELL.tool_name(), "k8s.node_shell");
        let interactive: Vec<_> = COMMANDS
            .iter()
            .filter(|m| m.interactive)
            .map(|m| m.id)
            .collect();
        assert_eq!(
            interactive,
            [
                CommandId::NODE_SHELL,
                CommandId::POD_ATTACH,
                CommandId::POD_DEBUG,
                CommandId::POD_EXEC,
                CommandId::POD_SHELL
            ],
            "the interactive tools are a reviewed allow-list"
        );
    }
}
