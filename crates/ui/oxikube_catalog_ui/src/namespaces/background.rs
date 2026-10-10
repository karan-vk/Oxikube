//! The selector's background work: what it sends to the service, how it loads, and how it
//! follows the session. All Kubernetes and state work goes through `spawn_kube`, off the UI
//! thread.

use gpui::{Context, SharedString, Subscription, Task};
use oxikube_app::SessionChange;
use oxikube_app::session::namespaces::Reconciled;
use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_runtime::{KubeTaskError, spawn_kube};
use oxikube_workspace::cluster::{EchoItem, SessionEcho, observe_session_echo};

use super::events::NamespaceSelectorEvent;
use super::selector::NamespaceSelector;

/// The task's result with the bridge's own failures folded into the error type.
pub(super) fn flatten<T>(result: Result<OxiResult<T>, KubeTaskError>) -> OxiResult<T> {
    result.map_err(OxiError::from).and_then(|inner| inner)
}

impl NamespaceSelector {
    pub(super) fn commit_debounced(
        &mut self,
        selection: NamespaceSelection,
        cx: &mut Context<Self>,
    ) {
        self.dirty = true;
        self.error = None;
        self.commit_generation += 1;
        let generation = self.commit_generation;
        let (service, cluster) = (self.service.clone(), self.cluster.clone());
        self.commit = Some(cx.spawn(async move |this, cx| {
            let result = flatten(
                spawn_kube(cx, async move {
                    service.select_debounced(&cluster, selection).await
                })
                .await,
            );
            this.update(cx, |this, cx| {
                if this.commit_generation == generation {
                    this.dirty = false;
                    // Whatever happened (applied, or superseded by a command or a prune), the
                    // session is the truth now; `NamespaceChanged`s that came while the tick
                    // was waiting were ignored.
                    this.resync_from_session(cx);
                }
                if let Err(err) = result {
                    this.failed(err, cx);
                }
            })
            .ok();
        }));
    }

    /// Sends `command` to the service off the UI thread. It runs to completion even if the
    /// view goes away: the user asked for it.
    pub(super) fn run(&mut self, command: Command, cx: &mut Context<Self>) {
        let service = self.service.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result =
                flatten(spawn_kube(cx, async move { service.execute(&command).await }).await);
            this.update(cx, |this, cx| this.answered(result, cx)).ok();
        })
        .detach();
    }

    /// Runs `namespace::Select` now (E05-P600): the session's selection changes in this update
    /// and its update is echoed to the views ([`SessionEcho`]), so the table narrows in the frame
    /// after the key; remembering it runs off the UI thread.
    pub(super) fn select_now(&mut self, command: Command, cx: &mut Context<Self>) {
        self.error = None;
        let echo = SessionEcho::begin(self.service.manager());
        let selected = self.service.execute_now(&command);
        echo.finish(cx);
        match selected {
            Ok(selected) => {
                let remember = selected.remember();
                cx.spawn(async move |this, cx| {
                    let result = flatten(spawn_kube(cx, remember).await);
                    this.update(cx, |this, cx| this.answered(result, cx)).ok();
                })
                .detach();
            }
            Err(err) => self.failed(err, cx),
        }
    }

    /// Follows the session echo: a selection changed on the UI thread shows in that update.
    pub(super) fn follow_session_echo(&mut self, cx: &mut Context<Self>) -> Subscription {
        observe_session_echo(cx, |this: &mut Self, items: &[EchoItem], cx| {
            let relevant = items.iter().any(|item| match item {
                Ok(update) => {
                    update.cluster == this.cluster
                        && matches!(update.change, SessionChange::NamespaceChanged(_))
                }
                Err(_) => true,
            });
            if relevant {
                this.session_changed(cx);
            }
        })
    }

    /// Shows a failed answer; a success needs nothing, the view already moved.
    pub(super) fn answered<T>(&mut self, result: OxiResult<T>, cx: &mut Context<Self>) {
        if let Err(err) = result {
            self.failed(err, cx);
        }
    }

    /// Shows `err` and goes back to what the service really has.
    pub(super) fn failed(&mut self, err: OxiError, cx: &mut Context<Self>) {
        let message = SharedString::from(err.message().to_owned());
        self.error = Some(message.clone());
        cx.emit(NamespaceSelectorEvent::Failed(message));
        let service = self.service.clone();
        let cluster = self.cluster.clone();
        cx.spawn(async move |this, cx| {
            let prefs = flatten(spawn_kube(cx, async move { service.prefs(&cluster).await }).await);
            this.update(cx, |this, cx| {
                if let Ok(prefs) = prefs {
                    this.prefs = prefs;
                    this.dirty = false;
                    this.sync_selection_from_session();
                    this.rebuild();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // --- loading and following the session -------------------------------------------

    pub(super) fn load(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let (service, cluster) = (self.service.clone(), self.cluster.clone());
        cx.spawn(async move |this, cx| {
            let result =
                flatten(spawn_kube(cx, async move { service.start(&cluster).await }).await);
            this.update(cx, |this, cx| this.loaded(result, cx)).ok();
        })
    }

    pub(super) fn loaded(&mut self, result: OxiResult<Reconciled>, cx: &mut Context<Self>) {
        match result {
            Ok(Reconciled {
                catalog,
                prefs,
                dropped,
            }) => {
                self.catalog = catalog;
                if self.dirty {
                    self.prefs.favourites = prefs.favourites;
                    self.prefs.typed = prefs.typed;
                } else {
                    self.prefs = prefs;
                }
                self.rebuild();
                if !dropped.is_empty() {
                    cx.emit(NamespaceSelectorEvent::StaleDropped(dropped));
                }
            }
            Err(err) => self.failed(err, cx),
        }
        cx.notify();
    }

    pub(super) fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        let (service, cluster) = (self.service.clone(), self.cluster.clone());
        cx.spawn(async move |this, cx| {
            let result =
                flatten(spawn_kube(cx, async move { service.catalog(&cluster).await }).await);
            this.update(cx, |this, cx| {
                if let Ok(catalog) = result {
                    this.catalog = catalog;
                    this.rebuild();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Follows `NamespaceChanged` for this cluster, so a selection changed by a command (the
    /// palette, an agent) shows here. While a tick is waiting, the view's own selection wins.
    pub(super) fn watch_session(&mut self, cx: &mut Context<Self>) -> Task<()> {
        use futures::StreamExt as _;
        let mut updates = self.service.manager().subscribe();
        let cluster = self.cluster.clone();
        cx.spawn(async move |this, cx| {
            while let Some(item) = updates.next().await {
                let relevant = match item {
                    Ok(update) if update.cluster != cluster => false,
                    Ok(update) => matches!(update.change, SessionChange::NamespaceChanged(_)),
                    // Missed updates: re-read below.
                    Err(_) => true,
                };
                if relevant
                    && this
                        .update(cx, |this, cx| this.session_changed(cx))
                        .is_err()
                {
                    break;
                }
            }
        })
    }

    pub(super) fn session_changed(&mut self, cx: &mut Context<Self>) {
        if self.dirty {
            return;
        }
        self.resync_from_session(cx);
    }

    /// Takes the session's selection and redraws if it differs from the view's.
    fn resync_from_session(&mut self, cx: &mut Context<Self>) {
        let before = self.prefs.selection.clone();
        self.sync_selection_from_session();
        if self.prefs.selection != before {
            self.rebuild();
            cx.notify();
        }
    }

    pub(super) fn sync_selection_from_session(&mut self) {
        if let Some(session) = self.service.manager().get(&self.cluster) {
            self.prefs.selection = session.namespace_selection().clone();
        }
    }
}
