//! The settings side of the sources screen: the stored list is `kubeconfig.sources` in
//! `settings.json`.
//!
//! [`SettingsSourceList`] is the [`SourceListStore`] the service reads and writes the list
//! through. Reading is a copy kept in step with the setting (so `load` never waits for the UI
//! thread); writing hands the new list to a task on the UI thread that edits the user's
//! `settings.json` with [`update_user_settings`] (comment-preserving, atomic) and replies when
//! the edit is applied. [`follow`] pushes every change of the setting, including one the user
//! typed into the file by hand, to the cluster source: that is the hot reload.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use gpui::{App, Subscription, Task};
use oxikube_app::{KubeconfigSourcesService, SourceListStore};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::UserSource;
use oxikube_runtime::spawn_kube;
use oxikube_settings::kubeconfig::{KubeconfigSettings, KubeconfigSourceEntry, default_sources};
use oxikube_settings::{Settings as _, SettingsStore, update_user_settings};
use parking_lot::Mutex;

/// One write of the list, waiting for the UI thread.
struct SaveRequest {
    sources: Vec<UserSource>,
    reply: oneshot::Sender<OxiResult<()>>,
}

/// The [`SourceListStore`] over the `kubeconfig.sources` setting. See the module docs.
pub struct SettingsSourceList {
    current: Arc<Mutex<Vec<UserSource>>>,
    saves: mpsc::UnboundedSender<SaveRequest>,
}

/// Keeps a [`SettingsSourceList`] alive: its observer and its writer task. Drop it and the list
/// stops following the setting.
pub struct SettingsSourceListHandle {
    list: Arc<SettingsSourceList>,
    _observe: Subscription,
    _writer: Task<()>,
}

impl SettingsSourceListHandle {
    /// The store, to give to the service.
    pub fn store(&self) -> Arc<SettingsSourceList> {
        self.list.clone()
    }
}

fn read(cx: &App) -> Vec<UserSource> {
    match KubeconfigSettings::try_get(cx) {
        Some(settings) => settings.user_sources(),
        None => KubeconfigSettings {
            sources: default_sources(),
        }
        .user_sources(),
    }
}

impl SettingsSourceList {
    /// Starts mirroring the setting. The settings store must be installed (it registers
    /// `KubeconfigSettings` itself); without one the list is the shipped default and writes
    /// fail with an error.
    pub fn install(cx: &mut App) -> SettingsSourceListHandle {
        let current = Arc::new(Mutex::new(read(cx)));
        let mirror = current.clone();
        let observe = KubeconfigSettings::observe(cx, move |cx| {
            *mirror.lock() = read(cx);
        });
        let (saves, mut requests) = mpsc::unbounded::<SaveRequest>();
        // Owned by the handle. It ends when the handle drops the sender's last clone.
        let writer = cx.spawn(async move |cx| {
            while let Some(request) = requests.next().await {
                let SaveRequest { sources, reply } = request;
                let entries: Vec<KubeconfigSourceEntry> = sources
                    .iter()
                    .map(KubeconfigSourceEntry::from_user_source)
                    .collect();
                let edit = cx.update(|cx| {
                    // Without a store there is nothing to edit (a tool or test run that never
                    // installed settings): say so instead of panicking on the missing global.
                    cx.has_global::<SettingsStore>().then(|| {
                        update_user_settings::<KubeconfigSettings>(cx, None, move |content| {
                            content.sources = Some(entries);
                        })
                    })
                });
                let result = match edit {
                    Some(task) => task.await,
                    None => Err(OxiError::unsupported(
                        "settings are not available, so the source list cannot be saved",
                    )),
                };
                let _ = reply.send(result);
            }
        });
        SettingsSourceListHandle {
            list: Arc::new(Self { current, saves }),
            _observe: observe,
            _writer: writer,
        }
    }
}

#[async_trait]
impl SourceListStore for SettingsSourceList {
    async fn load(&self) -> OxiResult<Vec<UserSource>> {
        Ok(self.current.lock().clone())
    }

    async fn save(&self, sources: &[UserSource]) -> OxiResult<()> {
        let (reply, answer) = oneshot::channel();
        self.saves
            .unbounded_send(SaveRequest {
                sources: sources.to_vec(),
                reply,
            })
            .map_err(|_| OxiError::internal("the settings writer has stopped"))?;
        answer
            .await
            .map_err(|_| OxiError::internal("the settings writer stopped before answering"))?
    }
}

/// Tells the cluster source about the list now and whenever the setting changes (a hot reload
/// of `settings.json`, or this screen's own edit). Keep the returned subscription.
pub fn follow(service: KubeconfigSourcesService, cx: &mut App) -> Subscription {
    apply(&service, cx);
    KubeconfigSettings::observe(cx, move |cx| apply(&service, cx))
}

fn apply(service: &KubeconfigSourcesService, cx: &mut App) {
    let service = service.clone();
    // Detached on purpose: a re-read of the sources runs to its end, and a newer change starts
    // its own. Nothing here is cleared from inside itself.
    cx.spawn(async move |cx| {
        let result = spawn_kube(&*cx, async move { service.apply_stored().await }).await;
        match result {
            Ok(Ok(changed)) if !changed.is_empty() => {
                tracing::debug!(
                    added = changed.added.len(),
                    "the kubeconfig sources changed"
                );
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::warn!(%error, "could not apply the kubeconfig sources"),
            Err(error) => tracing::warn!(%error, "the kubeconfig sources task failed"),
        }
    })
    .detach();
}
