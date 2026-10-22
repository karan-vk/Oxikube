//! The app-wide Helm controller: one `HelmCli`, one [`HelmGateState`], one task.
//!
//! The same `HelmCli` serves the repositories page and the charts catalog
//! ([`Installed::cli`]), so the `helm.path` setting, a hot reload and `helm::Recheck` reach every
//! Helm view, not only the gate.
//!
//! Detection runs `helm version` in a child process, so it goes through `spawn_kube` (the Tokio
//! bridge, aborted when dropped), never on the UI thread. The task waits for [`Request`]s and
//! keeps the gate in step: the first detection happens at start-up in the background, a change
//! of `helm.path` re-runs it (hot reload), and `helm::Recheck` runs it on demand. Requests that
//! queue up while a detection runs collapse into one more run.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Global, Subscription, Task, WeakEntity};
use oxikube_domain::helm::HelmCapabilities;
use oxikube_helm::{HelmCli, HelmCliConfig, TargetResolver};
use oxikube_helm_ui::HelmGateState;
use oxikube_runtime::RuntimeMode;
use oxikube_settings::{HelmSettings, Settings as _};

use super::{Request, Sink};

/// How a settings file is shown to the user: the system's editor unless a global of this type
/// says otherwise. The seam where tests see the file without launching an editor.
#[derive(Clone)]
pub struct FileOpener(pub Rc<dyn Fn(&Path, &mut App)>);

impl Global for FileOpener {}

/// A resolver that knows no cluster: for tests that only detect the binary.
#[cfg(test)]
pub(super) struct NoTargets;

#[cfg(test)]
impl TargetResolver for NoTargets {
    fn target(&self, _: &oxikube_domain::ids::ClusterId) -> Option<oxikube_helm::HelmTarget> {
        None
    }
}

/// Keeps the controller alive for the app (its task stops when dropped).
struct Running {
    installed: Installed,
    _task: Task<()>,
    _settings: Subscription,
}

impl Global for Running {}

/// The binary's configuration from the resolved `helm` setting.
fn cli_config(settings: &HelmSettings) -> HelmCliConfig {
    HelmCliConfig {
        path: settings.path.clone(),
        ..HelmCliConfig::default()
    }
}

/// What [`install`] hands the window.
#[derive(Clone)]
pub struct Installed {
    /// Where to send [`Request`]s.
    pub sink: Sink,
    /// The app's one `helm` adapter, configured from the `helm` setting and reconfigured on every
    /// change and recheck.
    pub cli: Arc<HelmCli>,
}

/// Starts the controller once for the app (later calls return the running one) and begins the
/// first detection in the background.
///
/// `targets` finds each cluster's kubeconfig context for the adapter's cluster commands
/// (installs, upgrades); only the first call's is used.
pub fn install(cx: &mut App, targets: Arc<dyn TargetResolver>) -> Installed {
    if let Some(running) = cx.try_global::<Running>() {
        return running.installed.clone();
    }
    let gate = HelmGateState::install(cx);
    // The releases page (E16-S05) reads the release records natively, with or without a binary.
    gate.update(cx, |gate, cx| gate.set_native_read(true, cx));
    // The task holds the gate weakly: a strong handle in a long-lived task is a leaked entity
    // when the app exits.
    let gate = gate.downgrade();
    // Configured from the setting already, so a view that uses the adapter before the first
    // detection finishes still looks where the user said.
    let cli = Arc::new(HelmCli::new(
        cli_config(&HelmSettings::resolve(cx)),
        targets,
    ));
    let installed_cli = cli.clone();
    let (sink, mut requests) = mpsc::unbounded::<Request>();
    let _ = sink.unbounded_send(Request::Recheck);
    let task = cx.spawn(async move |cx| {
        let mut path = None::<PathBuf>;
        while let Some(first) = requests.next().await {
            // Everything queued while a detection ran is handled together: one more run at
            // most, and a file to open is never dropped.
            let mut batch = vec![first];
            while let Ok(next) = requests.try_recv() {
                batch.push(next);
            }
            let (mut recheck, mut settings) = (false, false);
            for request in batch {
                match request {
                    Request::Open(file) => cx.update(|cx| open(&file, cx)),
                    Request::Recheck => recheck = true,
                    Request::SettingsChanged => settings = true,
                }
            }
            // Only the binary's path matters for a settings change; the other keys are read
            // where they are used.
            let moved = settings && cx.update(|cx| HelmSettings::resolve(cx).path) != path;
            if recheck || moved {
                detect(&cli, &gate, &mut path, cx).await;
            }
        }
    });
    let settings = HelmSettings::observe(cx, {
        let sink = sink.clone();
        move |_| {
            let _ = sink.unbounded_send(Request::SettingsChanged);
        }
    });
    let installed = Installed {
        sink,
        cli: installed_cli,
    };
    cx.set_global(Running {
        installed: installed.clone(),
        _task: task,
        _settings: settings,
    });
    installed
}

/// Runs one detection off the UI thread and publishes the answer.
async fn detect(
    cli: &Arc<HelmCli>,
    gate: &WeakEntity<HelmGateState>,
    last_path: &mut Option<PathBuf>,
    cx: &mut AsyncApp,
) {
    let config = cx.update(|cx| {
        gate.update(cx, |gate, cx| gate.begin_check(cx)).ok();
        cli_config(&HelmSettings::resolve(cx))
    });
    *last_path = config.path.clone();
    let cli = cli.clone();
    // The test runtime has no tokio reactor to spawn a process on: nothing is found there, and
    // the tests that need a binary run on the real runtime.
    let task = cx.update(|cx| {
        (oxikube_runtime::mode(cx) != Some(RuntimeMode::Deterministic))
            .then(|| oxikube_runtime::spawn_kube(cx, async move { cli.reconfigure(config).await }))
    });
    let found = match task {
        Some(task) => task.await.unwrap_or_else(|error| {
            tracing::error!(%error, "helm detection did not finish");
            HelmCapabilities::none()
        }),
        None => HelmCapabilities::none(),
    };
    cx.update(|cx| {
        gate.update(cx, |gate, cx| gate.set_cli_capabilities(found, cx))
            .ok();
    });
}

fn open(file: &Path, cx: &mut App) {
    match cx.try_global::<FileOpener>().cloned() {
        Some(opener) => (opener.0)(file, cx),
        None => cx.open_with_system(file),
    }
}
