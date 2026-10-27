//! Test doubles for the posture commands: a [`PrefsWriter`] that behaves like the settings store
//! (it keeps the written values and, like the hot reload in the binary, pushes them back into
//! the session manager), and the command samples the enforcement tests iterate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::{BoxFuture, FutureExt};
use oxikube_domain::command::{Command, CommandId, KubeconfigSourceRef, NewKubeconfigSource};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{ClusterColour, ClusterPreset, ErrorKind, OxiError, OxiResult};
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable};
use parking_lot::Mutex;

use crate::guard::{PrefsPatch, PrefsWriter};
use crate::session::ClusterSessionManager;
use crate::testing::{id, node, pod};

/// A ClusterRole the posture samples of `role::Create` and `role::Edit` carry.
const ROLE_MANIFEST: &str = r#"{"apiVersion":"rbac.authorization.k8s.io/v1","kind":"ClusterRole","metadata":{"name":"reader"},"rules":[{"apiGroups":[""],"resources":["pods"],"verbs":["get"]}]}"#;

/// A fake settings store.
pub(crate) struct FakePrefsWriter {
    manager: ClusterSessionManager,
    values: Mutex<HashMap<ClusterId, ClusterPrefs>>,
    writes: Mutex<Vec<(ClusterId, Option<String>, PrefsPatch)>>,
    /// Fail the next writes with an internal error.
    pub(crate) fail: AtomicBool,
    /// Push the written values back into the manager, like the hot reload does.
    pub(crate) echo: AtomicBool,
}

impl FakePrefsWriter {
    pub(crate) fn new(manager: ClusterSessionManager) -> Self {
        Self {
            manager,
            values: Mutex::default(),
            writes: Mutex::default(),
            fail: AtomicBool::new(false),
            echo: AtomicBool::new(true),
        }
    }

    /// Every write, in order: cluster, name hint, patch.
    pub(crate) fn writes(&self) -> Vec<(ClusterId, Option<String>, PrefsPatch)> {
        self.writes.lock().clone()
    }

    /// The stored settings of `cluster`.
    pub(crate) fn stored(&self, cluster: &ClusterId) -> ClusterPrefs {
        self.values.lock().get(cluster).cloned().unwrap_or_default()
    }

    /// Pre-seeds `cluster`'s settings (as if loaded from `settings.json`) and pushes them.
    pub(crate) fn seed(&self, cluster: &ClusterId, prefs: ClusterPrefs) {
        self.values.lock().insert(cluster.clone(), prefs);
        self.push();
    }

    fn push(&self) {
        let mut table = ClusterPrefsTable::new(ClusterPrefs::default());
        for (cluster, prefs) in self.values.lock().iter() {
            table = table.with_cluster(cluster.clone(), prefs.clone());
        }
        self.manager.set_prefs_table(table);
    }
}

impl PrefsWriter for FakePrefsWriter {
    fn write(
        &self,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        patch: PrefsPatch,
    ) -> BoxFuture<'static, OxiResult<()>> {
        self.writes
            .lock()
            .push((cluster.clone(), name_hint.map(str::to_owned), patch));
        if self.fail.load(Ordering::SeqCst) {
            return futures::future::ready(Err(OxiError::new(ErrorKind::Internal, "disk full")))
                .boxed();
        }
        {
            let mut values = self.values.lock();
            let prefs = values.entry(cluster.clone()).or_default();
            if let Some(read_only) = patch.read_only {
                prefs.read_only = read_only;
            }
            if let Some(colour) = patch.colour {
                prefs.colour = colour;
            }
        }
        if self.echo.load(Ordering::SeqCst) {
            self.push();
        }
        futures::future::ready(Ok(())).boxed()
    }
}

/// The production colour, for tests.
pub(crate) const RED: ClusterColour = ClusterPreset::PROD_COLOUR;

/// A sample of every declared command, aimed at cluster `name`.
///
/// The read-only enforcement test iterates every declared command and needs one payload for
/// each. A command added to `oxikube_domain::command` without a sample here fails that test
/// with the message below, so a new mutating command can never ship unchecked.
pub(crate) fn sample(command: CommandId, name: &str) -> Command {
    let cluster = id(name);
    let manifest = || "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: x\n".to_owned();
    let target = || pod(name, "web-0");
    match command.as_str() {
        "app::Quit" => Command::AppQuit,
        "app::SendTestNotification" => Command::AppSendTestNotification,
        "helm::Recheck" => Command::HelmRecheck,
        "helm::LocateBinary" => Command::HelmLocateBinary,
        "app::ToggleOsNotifications" => Command::AppToggleOsNotifications { enabled: None },
        "notifications::MarkAllRead" => Command::NotificationsMarkAllRead,
        "portforward::Start" => Command::PortForwardStart {
            target: target(),
            remote_port: oxikube_domain::portforward::ForwardPort::Number(80),
            local_port: None,
            bind: None,
            name: None,
            favourite: false,
        },
        "portforward::FavouriteAdd" => Command::PortForwardFavouriteAdd {
            target: target(),
            remote_port: oxikube_domain::portforward::ForwardPort::Number(80),
            local_port: None,
            bind: None,
            name: None,
            auto_start: None,
        },
        "portforward::FavouriteRemove" => {
            Command::PortForwardFavouriteRemove { id: "fav-1".into() }
        }
        "portforward::FavouriteToggleAutoStart" => {
            Command::PortForwardFavouriteToggleAutoStart { id: "fav-1".into() }
        }
        "portforward::Stop" => Command::PortForwardStop { id: 1 },
        "portforward::Restart" => Command::PortForwardRestart { id: 1 },
        "portforward::Remove" => Command::PortForwardRemove { id: 1 },
        "portforward::CopyUrl" => Command::PortForwardCopyUrl { id: 1 },
        "portforward::OpenInBrowser" => Command::PortForwardOpenInBrowser { id: 1 },
        "portforward::TogglePanel" => Command::PortForwardTogglePanel { show: false },
        "notifications::OpenLink" => Command::NotificationsOpenLink {
            url: "https://example.com".to_owned(),
        },
        "notifications::MarkRead" => Command::NotificationsMarkRead { id: 1 },
        "notifications::MuteCluster" => Command::NotificationsMuteCluster { cluster },
        "notifications::TogglePanel" => Command::NotificationsTogglePanel,
        "notifications::UnmuteCluster" => Command::NotificationsUnmuteCluster { cluster },
        "cluster::ApplyPreset" => Command::ClusterApplyPreset {
            cluster,
            preset: ClusterPreset::Staging,
        },
        "cluster::CancelConnect" => Command::ClusterCancelConnect { cluster },
        "cluster::CloseTab" => Command::ClusterCloseTab { cluster },
        "cluster::Connect" => Command::ClusterConnect { cluster },
        "cluster::Disconnect" => Command::ClusterDisconnect { cluster },
        "cluster::NextTab" => Command::ClusterNextTab,
        "cluster::PreviousTab" => Command::ClusterPreviousTab,
        "cluster::Reconnect" => Command::ClusterReconnect { cluster },
        "cluster::RedetectMetrics" => Command::ClusterRedetectMetrics { cluster },
        "cluster::Select" => Command::ClusterSelect { cluster },
        "cluster::SetColour" => Command::ClusterSetColour {
            cluster,
            colour: Some(RED),
        },
        "cluster::ShowMetricsSettings" => Command::ClusterShowMetricsSettings { cluster },
        "helm::ShowRepos" => Command::HelmShowRepos,
        "helm::OpenReleases" => Command::HelmOpenReleases { cluster },
        "rbac::OpenUsers" => Command::RbacOpenUsers { cluster },
        "rbac::OpenGroups" => Command::RbacOpenGroups { cluster },
        "rbac::ShowSubject" => Command::RbacShowSubject {
            cluster,
            subject: "system:serviceaccount:default:builder".to_owned(),
        },
        "rbac::CopyMatrix" => Command::RbacCopyMatrix { cluster },
        "helm::ShowValues" => Command::HelmShowValues {
            cluster,
            namespace: "apps".to_owned(),
            name: "web".to_owned(),
            revision: None,
            all: false,
        },
        "helm::ShowManifest" => Command::HelmShowManifest {
            cluster,
            namespace: "apps".to_owned(),
            name: "web".to_owned(),
            revision: None,
        },
        "helm::RevealValues" => Command::HelmRevealValues {
            cluster,
            namespace: "apps".to_owned(),
            name: "web".to_owned(),
            reveal: true,
        },
        "helm::OpenCharts" => Command::HelmOpenCharts,
        "helm::SearchCharts" => Command::HelmSearchCharts {
            keyword: "nginx".to_owned(),
        },
        "helm::ShowChart" => Command::HelmShowChart {
            repo: "bitnami".to_owned(),
            name: "nginx".to_owned(),
            version: None,
            part: oxikube_domain::helm::ChartPart::Readme,
        },
        "helm::InstallChart" => Command::HelmInstallChart {
            repo: "bitnami".to_owned(),
            name: "nginx".to_owned(),
            version: None,
        },
        "helm::UpgradeRelease" => Command::HelmUpgradeRelease {
            namespace: Some("default".to_owned()),
            release: Some("web".to_owned()),
        },
        "helm::Upgrade" => Command::HelmUpgrade {
            cluster,
            namespace: "default".to_owned(),
            release: "web".to_owned(),
            repo: Some("bitnami".to_owned()),
            chart: "nginx".to_owned(),
            version: None,
            values: oxikube_domain::helm::ValuesYaml::default(),
            reuse_values: false,
            wait: false,
            timeout_secs: None,
        },
        "helm::Install" => Command::HelmInstall {
            cluster,
            release: "web".to_owned(),
            namespace: "default".to_owned(),
            create_namespace: false,
            repo: Some("bitnami".to_owned()),
            chart: "nginx".to_owned(),
            version: None,
            values: oxikube_domain::helm::ValuesYaml::default(),
            wait: false,
            timeout_secs: None,
        },
        "role::OpenDialog" => Command::RoleOpenDialog {
            cluster: Some(cluster),
            cluster_role: false,
            target: None,
        },
        "role::Create" => Command::RoleCreate {
            cluster,
            manifest: ROLE_MANIFEST.to_owned(),
        },
        "role::Edit" => Command::RoleEdit {
            cluster,
            manifest: ROLE_MANIFEST.to_owned(),
        },
        "role::Delete" => Command::RoleDelete {
            target: ResourceRef::cluster_scoped(
                cluster,
                oxikube_domain::rbac::RbacKind::ClusterRole.gvk(),
                "reader",
            ),
            bindings: None,
        },
        "helm::Rollback" => Command::HelmRollback {
            cluster,
            namespace: "default".to_owned(),
            release: "web".to_owned(),
            revision: 1,
        },
        "helm::Uninstall" => Command::HelmUninstall {
            cluster,
            namespace: "default".to_owned(),
            release: "web".to_owned(),
            keep_history: false,
        },
        "helm::RepoAdd" => Command::HelmRepoAdd {
            name: "bitnami".to_owned(),
            url: "https://charts.bitnami.com/bitnami".to_owned(),
            username: None,
            insecure_skip_tls_verify: false,
            ca_file: None,
        },
        "helm::RepoRemove" => Command::HelmRepoRemove {
            name: "bitnami".to_owned(),
        },
        "helm::RepoUpdate" => Command::HelmRepoUpdate { names: Vec::new() },
        "cluster::ShowPulses" => Command::ClusterShowPulses { cluster },
        "rbac::CanI" => Command::RbacCanI {
            cluster,
            target: None,
            verb: None,
        },
        "events::Open" => Command::EventsOpen {
            cluster,
            namespace: None,
            filter: None,
        },
        "events::ToggleFaults" => Command::EventsToggleFaults { cluster },
        "events::TogglePause" => Command::EventsTogglePause { cluster },
        "events::CopyRow" => Command::EventsCopyRow { cluster },
        "events::CopyRowJson" => Command::EventsCopyRowJson { cluster },
        "events::ExportJson" => Command::EventsExportJson { cluster },
        "cluster::ShowOverview" => Command::ClusterShowOverview { cluster },
        "events::SendToAgent" => Command::EventsSendToAgent {
            cluster,
            target: None,
            events: Vec::new(),
        },
        "cluster::SwitchTab" => Command::ClusterSwitchTab { index: 1 },
        "cluster::ToggleFavourite" => Command::ClusterToggleFavourite {
            cluster,
            favourite: None,
        },
        "cluster::ToggleMetrics" => Command::ClusterToggleMetrics {
            cluster,
            enabled: None,
        },
        "cluster::ToggleReadOnly" => Command::ClusterToggleReadOnly {
            cluster,
            read_only: None,
        },
        "kubeconfig::AddSource" => Command::KubeconfigAddSource {
            source: NewKubeconfigSource::File {
                path: "/tmp/kubeconfig".into(),
            },
        },
        "kubeconfig::Reload" => Command::KubeconfigReload,
        "kubeconfig::RemoveSource" => Command::KubeconfigRemoveSource {
            source: KubeconfigSourceRef::Default,
        },
        "logs::Clear" => Command::LogsClear { target: target() },
        "logs::Copy" => Command::LogsCopy { target: target() },
        "logs::Mark" => Command::LogsMark { target: target() },
        "logs::SendToAgent" => Command::LogsSendToAgent { target: target() },
        "logs::TailInTerminal" => Command::LogsTailInTerminal { target: target() },
        "logs::FollowReplacement" => Command::LogsFollowReplacement { target: target() },
        "logs::Reconnect" => Command::LogsReconnect { target: target() },
        "logs::Save" => Command::LogsSave {
            target: target(),
            scope: oxikube_domain::log::LogSaveScope::All,
        },
        "logs::SelectContainer" => Command::LogsSelectContainer {
            target: target(),
            container: "app".into(),
        },
        "logs::SetRange" => Command::LogsSetRange {
            target: target(),
            range: oxikube_domain::log::LogRange::Last5m,
        },
        "logs::ToggleAutoscroll" => Command::LogsToggleAutoscroll { target: target() },
        "logs::ToggleFullscreen" => Command::LogsToggleFullscreen { target: target() },
        "logs::TogglePrevious" => Command::LogsTogglePrevious { target: target() },
        "logs::ToggleSource" => Command::LogsToggleSource {
            target: target(),
            pod: "web-0".into(),
            container: None,
        },
        "logs::ToggleTimestamps" => Command::LogsToggleTimestamps { target: target() },
        "logs::ToggleWrap" => Command::LogsToggleWrap { target: target() },
        "logs::CloseSearch" => Command::LogsCloseSearch { target: target() },
        "logs::Find" => Command::LogsFind {
            target: target(),
            pattern: Some("error".into()),
        },
        "logs::NextMatch" => Command::LogsNextMatch { target: target() },
        "logs::PreviousMatch" => Command::LogsPreviousMatch { target: target() },
        "logs::ToggleCase" => Command::LogsToggleCase { target: target() },
        "logs::ToggleFilterMode" => Command::LogsToggleFilterMode { target: target() },
        "logs::ToggleInverse" => Command::LogsToggleInverse { target: target() },
        "logs::ToggleJsonMode" => Command::LogsToggleJsonMode { target: target() },
        "logs::ToggleLevel" => Command::LogsToggleLevel {
            target: target(),
            level: oxikube_domain::log::LevelChip::Error,
        },
        "logs::ToggleLine" => Command::LogsToggleLine {
            target: target(),
            seq: 0,
        },
        "logs::CollapseLine" => Command::LogsCollapseLine { target: target() },
        "namespace::Select" => Command::NamespaceSelect {
            cluster,
            namespaces: vec!["default".into()],
        },
        "namespace::ToggleFavourite" => Command::NamespaceToggleFavourite {
            cluster,
            namespace: "default".into(),
        },
        "namespace::UseFavourite" => Command::NamespaceUseFavourite { cluster, slot: 1 },
        "namespace::Create" => Command::NamespaceCreate {
            cluster,
            name: "team-a".into(),
            labels: Default::default(),
        },
        "namespace::Delete" => Command::NamespaceDelete {
            target: ResourceRef::cluster_scoped(cluster, Gvk::new("", "v1", "Namespace"), "team-a"),
            contents: Default::default(),
        },
        "node::Cordon" => Command::NodeCordon {
            target: node(name, "worker-1"),
        },
        "node::Drain" => Command::NodeDrain {
            target: node(name, "worker-1"),
            flags: oxikube_domain::drain::DrainFlags::default(),
        },
        "node::Shell" => Command::NodeShell {
            target: node(name, "worker-1"),
        },
        "ingressclass::SetDefault" => Command::IngressClassSetDefault {
            target: ResourceRef::cluster_scoped(
                id(name),
                Gvk::new("networking.k8s.io", "v1", "IngressClass"),
                "nginx",
            ),
            clear_previous: true,
        },
        "node::Uncordon" => Command::NodeUncordon {
            target: node(name, "worker-1"),
        },
        "palette::Toggle" => Command::PaletteToggle,
        "pvc::Resize" => Command::PvcResize {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("", "v1", "PersistentVolumeClaim"),
                "default",
                "data",
            ),
            size: "20Gi".into(),
            from: Some("10Gi".into()),
        },
        "storageclass::SetDefault" => Command::StorageClassSetDefault {
            target: ResourceRef::cluster_scoped(
                cluster,
                Gvk::new("storage.k8s.io", "v1", "StorageClass"),
                "fast",
            ),
            clear_previous: true,
        },
        "palette::ClearRecents" => Command::PaletteClearRecents,
        "palette::OpenJump" => Command::PaletteOpenJump,
        "help::Show" => Command::HelpShow,
        "palette::ToggleShowAll" => Command::PaletteToggleShowAll,
        "jump::Back" => Command::JumpBack,
        "jump::Forward" => Command::JumpForward,
        "jump::Last" => Command::JumpLast,
        "keymap::OpenUser" => Command::KeymapOpenUser,
        "charts::OpenGallery" => Command::ChartsOpenGallery,
        "pod::Delete" => Command::PodDelete {
            target: target(),
            grace_period_seconds: None,
        },
        "pod::Evict" => Command::PodEvict { target: target() },
        "pod::ForceDelete" => Command::PodForceDelete { target: target() },
        "pod::Sanitize" => Command::PodSanitize {
            cluster,
            namespace: Some("default".into()),
            targets: vec![target()],
        },
        "pod::JumpToOwner" => Command::PodJumpToOwner { target: target() },
        "pod::ShowNode" => Command::PodShowNode { target: target() },
        "pod::Debug" => Command::PodDebug {
            target: target(),
            image: "busybox".into(),
            target_container: None,
            command: Vec::new(),
            name: None,
        },
        "pod::Attach" => Command::PodAttach {
            target: target(),
            container: None,
        },
        "pod::Shell" => Command::PodShell {
            target: target(),
            container: None,
        },
        "pod::Exec" => Command::PodExec {
            target: target(),
            container: None,
            command: vec!["sh".into()],
        },
        "pod::PortForward" => Command::PodPortForward {
            target: target(),
            local_port: None,
            remote_port: 8080,
        },
        "service::OpenInBrowser" => Command::ServiceOpenInBrowser {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("", "v1", "Service"),
                "default",
                "web",
            ),
            port: Some(80),
        },
        "resource::ShowUsedBy" => Command::ResourceShowUsedBy { target: target() },
        "service::ShowPods" => Command::ServiceShowPods {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("", "v1", "Service"),
                "default",
                "web",
            ),
        },
        "pod::ViewLogs" => Command::PodViewLogs {
            target: target(),
            container: None,
            follow: false,
            previous: false,
            tail_lines: None,
        },
        "resource::Apply" => Command::ResourceApply {
            cluster,
            namespace: None,
            manifest: manifest(),
            field_manager: None,
            force: false,
            resource_version: None,
            reviewed_absent: false,
        },
        "resource::Delete" => Command::ResourceDelete {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
            propagation: oxikube_domain::Propagation::Background,
        },
        "resource::Open" => Command::ResourceOpen { target: target() },
        "resource::OpenList" => Command::ResourceOpenList {
            cluster,
            gvk: Gvk::new("", "v1", "Pod"),
        },
        "crd::OpenList" => Command::CrdOpenList { cluster },
        "crd::OpenResources" => Command::CrdOpenResources {
            cluster,
            name: "widgets.example.com".into(),
        },
        "resource::CopyLabel" => Command::ResourceCopyLabel {
            target: target(),
            key: "app".into(),
            annotation: false,
        },
        "resource::CopyName" => Command::ResourceCopyName { target: target() },
        "resource::CopyYaml" => Command::ResourceCopyYaml { target: target() },
        "resource::RefreshDescribe" => Command::ResourceRefreshDescribe { target: target() },
        "resource::SaveYaml" => Command::ResourceSaveYaml { target: target() },
        "resource::Find" => Command::ResourceFind {
            target: target(),
            pattern: Some("image".into()),
        },
        "resource::NextMatch" => Command::ResourceNextMatch { target: target() },
        "resource::PreviousMatch" => Command::ResourcePreviousMatch { target: target() },
        "resource::ToggleManagedFields" => {
            Command::ResourceToggleManagedFields { target: target() }
        }
        "resource::RetryFeed" => Command::ResourceRetryFeed {
            cluster,
            gvk: Gvk::new("", "v1", "Pod"),
        },
        "resource::PinDetail" => Command::ResourcePinDetail { target: target() },
        "resource::SelectAll" => Command::ResourceSelectAll {
            cluster,
            gvk: Gvk::new("", "v1", "Pod"),
        },
        "table::FocusFilter" => Command::TableFocusFilter {
            cluster,
            gvk: Gvk::new("", "v1", "Pod"),
        },
        "table::SetFilter" => Command::TableSetFilter {
            cluster,
            gvk: Gvk::new("", "v1", "Pod"),
            text: "/web".to_owned(),
        },
        "resource::ViewYaml" => Command::ResourceViewYaml { target: target() },
        "resource::ViewDescribe" => Command::ResourceViewDescribe { target: target() },
        "resource::ViewEvents" => Command::ResourceViewEvents { target: target() },
        "resource::ViewMetrics" => Command::ResourceViewMetrics { target: target() },
        "resource::Edit" => Command::ResourceEdit { target: target() },
        "secret::Reveal" => Command::SecretReveal {
            target: target(),
            key: None,
        },
        "secret::EditDecoded" => Command::SecretEditDecoded { target: target() },
        "secret::Create" => Command::SecretCreate { cluster: None },
        "table::ToggleWide" => Command::TableToggleWide {
            cluster,
            gvk: Gvk::new("", "v1", "Pod"),
        },
        "view::Open" => Command::ViewOpen {
            view: "overview".into(),
        },
        "view::ZoomIn" => Command::ViewZoomIn,
        "view::ZoomOut" => Command::ViewZoomOut,
        "view::ZoomReset" => Command::ViewZoomReset,
        "terminal::Copy" => Command::TerminalCopy,
        "terminal::Paste" => Command::TerminalPaste,
        "terminal::New" => Command::TerminalNew { cluster: None },
        "editor::NewManifest" => Command::EditorNewManifest { cluster: None },
        "resource::Create" => Command::ResourceCreate {
            cluster: None,
            template: None,
        },
        "editor::ToggleManagedFields" => Command::EditorToggleManagedFields,
        "editor::ToggleSecretMask" => Command::EditorToggleSecretMask,
        "resource::ViewYamlInEditor" => Command::ResourceViewYamlInEditor { target: target() },
        "editor::ToggleReadOnly" => Command::EditorToggleReadOnly,
        "editor::ToggleSoftWrap" => Command::EditorToggleSoftWrap,
        "editor::ConvertToJson" => Command::EditorConvertToJson,
        "editor::ConvertToYaml" => Command::EditorConvertToYaml,
        "editor::UseJson" => Command::EditorUseJson,
        "editor::UseYaml" => Command::EditorUseYaml,
        "editor::ShowDiff" => Command::EditorShowDiff,
        "editor::ToggleDiffLayout" => Command::EditorToggleDiffLayout,
        "editor::ReloadFromServer" => Command::EditorReloadFromServer,
        "editor::MergeFromServer" => Command::EditorMergeFromServer,
        "editor::KeepMine" => Command::EditorKeepMine,
        "editor::TakeMine" => Command::EditorTakeMine { conflict: 0 },
        "editor::TakeServer" => Command::EditorTakeServer { conflict: 0 },
        "editor::AcceptMerge" => Command::EditorAcceptMerge,
        "editor::TriggerCompletion" => Command::EditorTriggerCompletion,
        "editor::ShowHover" => Command::EditorShowHover,
        "terminal::Split" => Command::TerminalSplit,
        "terminal::Close" => Command::TerminalClose,
        "terminal::Reconnect" => Command::TerminalReconnect,
        "terminal::Restart" => Command::TerminalRestart,
        "terminal::Clear" => Command::TerminalClear,
        "terminal::ScrollLineDown" => Command::TerminalScrollLineDown,
        "terminal::ScrollLineUp" => Command::TerminalScrollLineUp,
        "terminal::ScrollPageDown" => Command::TerminalScrollPageDown,
        "terminal::ScrollPageUp" => Command::TerminalScrollPageUp,
        "terminal::Search" => Command::TerminalSearch,
        "terminal::SearchClose" => Command::TerminalSearchClose,
        "terminal::SearchNext" => Command::TerminalSearchNext,
        "terminal::SearchPrevious" => Command::TerminalSearchPrevious,
        "terminal::SelectAll" => Command::TerminalSelectAll,
        "terminal::OpenLink" => Command::TerminalOpenLink {
            target: "https://kubernetes.io".into(),
        },
        "selection::Delete" => Command::SelectionDelete {
            targets: vec![target()],
            propagation: oxikube_domain::command::Propagation::Background,
        },
        "selection::Restart" => Command::SelectionRestart {
            targets: vec![ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            )],
        },
        "selection::Label" => Command::SelectionLabel {
            targets: vec![target()],
            field: oxikube_domain::selection::LabelField::Labels,
            edits: vec![oxikube_domain::selection::LabelEdit::set("app", "web")],
        },
        "window::New" => Command::WindowNew,
        "workload::Restart" => Command::WorkloadRestart {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
        },
        "workload::Rollback" => Command::WorkloadRollback {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
            to_revision: None,
        },
        "workload::SetImage" => Command::WorkloadSetImage {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
            images: vec![oxikube_domain::command::ImageChange::new(
                "web",
                "nginx:1.27",
            )],
        },
        "workload::Scale" => Command::WorkloadScale {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
            replicas: 3,
        },
        "workload::ViewLogs" => Command::WorkloadViewLogs {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
            selector: None,
            container: None,
            follow: true,
            tail_lines: None,
        },
        "cronjob::Trigger" => Command::CronJobTrigger {
            target: batch(name, "CronJob", "nightly"),
        },
        "cronjob::Suspend" => Command::CronJobSuspend {
            target: batch(name, "CronJob", "nightly"),
        },
        "cronjob::Resume" => Command::CronJobResume {
            target: batch(name, "CronJob", "nightly"),
        },
        "job::Suspend" => Command::JobSuspend {
            target: batch(name, "Job", "import"),
        },
        "job::Resume" => Command::JobResume {
            target: batch(name, "Job", "import"),
        },
        other => panic!(
            "no sample for the declared command {other}: add one to testing_posture::sample; the \
             read-only enforcement test iterates every declared command"
        ),
    }
}

/// A `batch/v1` object in `default` of cluster `name`.
fn batch(name: &str, kind: &str, object: &str) -> ResourceRef {
    ResourceRef::namespaced(id(name), Gvk::new("batch", "v1", kind), "default", object)
}
