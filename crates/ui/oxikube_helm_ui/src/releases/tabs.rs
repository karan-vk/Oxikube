//! Reading the detail's tabs: one read per tab, off the UI thread, applied when it lands.

use std::sync::Arc;
use std::time::Duration;

use gpui::Context;
use oxikube_app::helm_releases::{ManifestView, ValuesView};
use oxikube_domain::OxiResult;
use oxikube_domain::helm::Revision;
use oxikube_runtime::{KubeTaskError, notify_coalesced, spawn_kube};

use super::backend::ObjectsLease;
use super::detail::ValuesFacts;
use super::model::{Load, Tab, links};
use super::view::{HelmReleasesView, describe};

/// How often the Resources tab reads the objects' state while it shows.
pub const OBJECTS_TICK: Duration = Duration::from_secs(1);

/// The links offered under the notes.
const MAX_LINKS: usize = 8;

fn flatten<T>(result: Result<OxiResult<T>, KubeTaskError>) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(describe(&error)),
        Err(error) => Err(error.to_string()),
    }
}

impl HelmReleasesView {
    /// Reads the shown tab if it has not been read.
    pub(super) fn load_shown(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.detail.as_ref().map(|d| d.tab) {
            self.load_tab(tab, cx);
        }
    }

    fn load_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        if *detail.load(tab) != Load::Idle {
            return;
        }
        *detail.load_mut(tab) = Load::Loading;
        let (release, revision, updated) = (
            detail.release.release.clone(),
            detail.release.revision,
            detail.release.updated,
        );
        let generation = detail.generation;
        let all = detail.mode.all();
        let backend = self.deps.backend.clone();
        let task = match tab {
            Tab::Resources => {
                let read = backend.objects(&release, revision, updated);
                cx.spawn(async move |this, cx| {
                    let result = flatten(spawn_kube(cx, read).await);
                    this.update(cx, |this, cx| this.objects_arrived(generation, result, cx))
                        .ok();
                })
            }
            Tab::Values => {
                let read = backend.values(&release, revision, all);
                cx.spawn(async move |this, cx| {
                    let result = flatten(spawn_kube(cx, read).await);
                    this.update(cx, |this, cx| this.values_arrived(generation, result, cx))
                        .ok();
                })
            }
            Tab::Manifest => {
                let read = backend.manifest(&release, revision);
                cx.spawn(async move |this, cx| {
                    let result = flatten(spawn_kube(cx, read).await);
                    this.update(cx, |this, cx| this.manifest_arrived(generation, result, cx))
                        .ok();
                })
            }
            Tab::Notes => {
                let read = backend.notes(&release, revision);
                cx.spawn(async move |this, cx| {
                    let result = flatten(spawn_kube(cx, read).await);
                    this.update(cx, |this, cx| this.notes_arrived(generation, result, cx))
                        .ok();
                })
            }
            Tab::History => {
                let read = backend.history(&release);
                cx.spawn(async move |this, cx| {
                    let result = flatten(spawn_kube(cx, read).await);
                    this.update(cx, |this, cx| this.history_arrived(generation, result, cx))
                        .ok();
                })
            }
        };
        detail.tasks.insert(tab, task);
    }

    fn objects_arrived(
        &mut self,
        generation: u64,
        result: Result<Box<dyn ObjectsLease>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match result {
            Ok(lease) => {
                detail.resources.objects = lease.objects();
                detail.resources.lease = Some(lease);
                detail.resources.load = Load::Ready;
                if detail.tab == Tab::Resources {
                    self.start_ticking(cx);
                }
            }
            Err(why) => detail.resources.load = Load::Failed(why),
        }
        cx.notify();
    }

    /// While the Resources tab shows, the objects' state is read again every second: a cheap join
    /// of the plan with the store (no I/O), published only when it changed. The store's feeds
    /// keep the objects current; this is only the redraw cue the lease does not send.
    pub(super) fn start_ticking(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        if detail.resources.tick.is_some() || detail.resources.lease.is_none() {
            return;
        }
        detail.resources.tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(OBJECTS_TICK).await;
                if this.update(cx, |this, cx| this.read_objects(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    pub(super) fn read_objects(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        let Some(lease) = &detail.resources.lease else {
            return;
        };
        let objects = lease.objects();
        if objects != detail.resources.objects {
            detail.resources.objects = objects;
            notify_coalesced(cx);
        }
    }

    fn values_arrived(
        &mut self,
        generation: u64,
        result: Result<ValuesView, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match result {
            Ok(view) => {
                detail.values_facts = ValuesFacts {
                    sensitive: view.sensitive,
                    revealed: view.revealed,
                };
                detail.values.empty = view.empty;
                detail
                    .values
                    .code
                    .update(cx, |code, cx| code.set_text(view.text, cx));
                detail.values.load = Load::Ready;
            }
            Err(why) => detail.values.load = Load::Failed(why),
        }
        cx.notify();
    }

    fn manifest_arrived(
        &mut self,
        generation: u64,
        result: Result<ManifestView, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match result {
            Ok(view) => {
                detail.manifest_masked = view.masked;
                detail.manifest.empty = view.text.trim().is_empty();
                detail
                    .manifest
                    .code
                    .update(cx, |code, cx| code.set_text(view.text, cx));
                detail.manifest.load = Load::Ready;
            }
            Err(why) => detail.manifest.load = Load::Failed(why),
        }
        cx.notify();
    }

    fn notes_arrived(
        &mut self,
        generation: u64,
        result: Result<Option<Arc<str>>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match result {
            Ok(notes) => {
                let text = notes.unwrap_or_else(|| Arc::from(""));
                detail.notes.empty = text.trim().is_empty();
                detail.links = links(&text, MAX_LINKS);
                detail
                    .notes
                    .code
                    .update(cx, |code, cx| code.set_text(text, cx));
                detail.notes.load = Load::Ready;
            }
            Err(why) => detail.notes.load = Load::Failed(why),
        }
        cx.notify();
    }

    fn history_arrived(
        &mut self,
        generation: u64,
        result: Result<Vec<Revision>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.detail.as_mut().filter(|d| d.generation == generation) else {
            return;
        };
        match result {
            Ok(revisions) => {
                detail.history.revisions = revisions;
                detail.history.load = Load::Ready;
            }
            Err(why) => detail.history.load = Load::Failed(why),
        }
        cx.notify();
    }
}
