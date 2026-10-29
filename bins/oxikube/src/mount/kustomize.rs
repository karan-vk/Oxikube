//! Kustomize in the running app (E20-S04): the one `KustomizeCli`, its [`KustomizeState`] and
//! the `kustomize` capability.
//!
//! [`install`] builds the adapter from the `kustomize` setting and detects the tool in the
//! background (`spawn_kube`, never on the UI thread) shortly after start-up and again whenever
//! `kustomize.path` changes (hot reload, no restart). The answer lives in [`KustomizeState`]:
//! `None` while neither `kustomize` nor `kubectl kustomize` exists, which is how the apply UI
//! (E20-S05) decides to hide its "from kustomize" action. [`palette_capabilities`] feeds the
//! same answer to the palette as [`Capabilities::KUSTOMIZE`], so a command that needs it is
//! hidden (and explained under "show all") while no tool exists.
//!
//! A user reaches it through the `kustomize` block of `settings.json` and the apply panel's
//! "Kustomize folder" action (E20-S05); the service is [`Installed::service`].

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity, Global, Subscription, Task};
use oxikube_app::kustomize::KustomizeService;
use oxikube_domain::Capabilities;
use oxikube_domain::kustomize::{self, KustomizeTool};
use oxikube_kustomize::{KustomizeCli, KustomizeConfig};
use oxikube_ports::KustomizePort as _;
use oxikube_runtime::RuntimeMode;
use oxikube_settings::{KustomizeSettings, Settings as _};

/// What detection found, observable by views.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KustomizeState {
    /// The tool that builds overlays; `None` when neither binary exists (or before the first
    /// detection finishes).
    pub tool: Option<KustomizeTool>,
}

/// What [`install`] hands the window.
#[derive(Clone)]
pub struct Installed {
    /// Builds overlays through the app's one adapter.
    pub service: KustomizeService,
    /// The detected tool.
    pub state: Entity<KustomizeState>,
}

struct Running {
    installed: Installed,
    _task: Task<()>,
    _settings: Subscription,
}

impl Global for Running {}

fn config(settings: &KustomizeSettings) -> KustomizeConfig {
    KustomizeConfig {
        path: settings.path.clone(),
        ..KustomizeConfig::default()
    }
}

/// Starts the controller once for the app (later calls return the running one) and begins the
/// first detection in the background.
pub fn install(cx: &mut App) -> Installed {
    if let Some(running) = cx.try_global::<Running>() {
        return running.installed.clone();
    }
    let cli = Arc::new(KustomizeCli::new(config(&KustomizeSettings::resolve(cx))));
    let state = cx.new(|_| KustomizeState::default());
    let installed = Installed {
        service: KustomizeService::new(cli.clone()),
        state: state.clone(),
    };
    let (sink, mut requests) = mpsc::unbounded::<()>();
    let _ = sink.unbounded_send(());
    let weak = state.downgrade();
    let task = cx.spawn(async move |cx| {
        while requests.next().await.is_some() {
            // Requests queued while a detection ran collapse into one more run.
            while requests.try_recv().is_ok() {}
            let task = cx.update(|cx| {
                let settings = config(&KustomizeSettings::resolve(cx));
                // The test runtime has no tokio reactor to spawn a process on: nothing is found
                // there, and the tests that need a binary run on the real runtime.
                (oxikube_runtime::mode(cx) != Some(RuntimeMode::Deterministic)).then(|| {
                    let cli = cli.clone();
                    oxikube_runtime::spawn_kube(cx, async move {
                        cli.reconfigure(settings).await;
                        cli.detect().await
                    })
                })
            });
            let tool = match task {
                Some(task) => task.await.unwrap_or_else(|error| {
                    tracing::error!(%error, "kustomize detection did not finish");
                    None
                }),
                None => None,
            };
            cx.update(|cx| {
                weak.update(cx, |state, cx| {
                    if state.tool != tool {
                        state.tool = tool;
                        cx.notify();
                    }
                })
                .ok();
            });
        }
    });
    let subscription = KustomizeSettings::observe(cx, move |_| {
        let _ = sink.unbounded_send(());
    });
    cx.set_global(Running {
        installed: installed.clone(),
        _task: task,
        _settings: subscription,
    });
    installed
}

/// The capabilities the palette should treat the session as having: the session's own, with
/// [`Capabilities::KUSTOMIZE`] following whether a tool was detected.
pub fn palette_capabilities(cx: &App, session: Capabilities) -> Capabilities {
    let tool = cx
        .try_global::<Running>()
        .and_then(|running| running.installed.state.read(cx).tool.clone());
    kustomize::apply_to(session, tool.as_ref())
}
