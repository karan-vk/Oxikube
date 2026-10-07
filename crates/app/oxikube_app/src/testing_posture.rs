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
        "cluster::Select" => Command::ClusterSelect { cluster },
        "cluster::SetColour" => Command::ClusterSetColour {
            cluster,
            colour: Some(RED),
        },
        "cluster::SwitchTab" => Command::ClusterSwitchTab { index: 1 },
        "cluster::ToggleFavourite" => Command::ClusterToggleFavourite {
            cluster,
            favourite: None,
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
        "namespace::Select" => Command::NamespaceSelect {
            cluster,
            namespaces: vec!["default".into()],
        },
        "namespace::ToggleFavourite" => Command::NamespaceToggleFavourite {
            cluster,
            namespace: "default".into(),
        },
        "node::Cordon" => Command::NodeCordon {
            target: node(name, "worker-1"),
        },
        "node::Drain" => Command::NodeDrain {
            target: node(name, "worker-1"),
            force: false,
        },
        "node::Uncordon" => Command::NodeUncordon {
            target: node(name, "worker-1"),
        },
        "palette::Toggle" => Command::PaletteToggle,
        "pod::Delete" => Command::PodDelete {
            target: target(),
            grace_period_seconds: None,
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
        "resource::ViewYaml" => Command::ResourceViewYaml { target: target() },
        "view::Open" => Command::ViewOpen {
            view: "overview".into(),
        },
        "view::ZoomIn" => Command::ViewZoomIn,
        "view::ZoomOut" => Command::ViewZoomOut,
        "view::ZoomReset" => Command::ViewZoomReset,
        "terminal::OpenLink" => Command::TerminalOpenLink {
            target: "https://kubernetes.io".into(),
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
        "workload::Scale" => Command::WorkloadScale {
            target: ResourceRef::namespaced(
                cluster,
                Gvk::new("apps", "v1", "Deployment"),
                "default",
                "api",
            ),
            replicas: 3,
        },
        other => panic!(
            "no sample for the declared command {other}: add one to testing_posture::sample; the \
             read-only enforcement test iterates every declared command"
        ),
    }
}
