//! The editor's side of the live link: binding to an object, the watch's results applied through
//! [`LiveSync`](crate::diff::LiveSync), loading an object into the buffer. Everything that costs
//! more than a field write (rendering, normalising, finding the object) runs off the UI thread.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{AppContext as _, Context, SharedString, Window};
use oxikube_domain::Resource;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::ResourceReader;
use oxikube_runtime::notify_coalesced;
use oxikube_ui::editor::EditorApi as _;
use oxikube_workspace::ItemEvent;

use super::watch::{Watched, find_object, watch_object};
use super::{DIFF_DEBOUNCE, PanelBody};
use crate::diff::{
    LiveObject, LiveSync, ObjectId, ObjectVersion, Renderer, SyncAction, SyncState, document_for,
};
use crate::view::ManifestEditor;

/// Watch results queued before the editor takes them; only the latest is used.
const WATCH_QUEUE: usize = 8;

impl ManifestEditor {
    /// Loads `object` into the buffer and follows it: the buffer is then clean, a server change
    /// reloads it silently, and a server change while it is edited raises the banner. `render`
    /// writes the object as the buffer holds it (the open-from-resource renderer); `None` uses
    /// [`buffer_text`](crate::diff::buffer_text). The rendering runs off the UI thread.
    pub fn follow_object(
        &mut self,
        object: Resource,
        render: Option<Renderer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(render) = render {
            self.live.render = render;
        }
        let render = self.live.render.clone();
        self.live.load_seq += 1;
        let seq = self.live.load_seq;
        // Detached, not stored: a lookup in flight calls this from its own task. A newer load
        // makes this one stale (`load_seq`); it holds the editor weakly and ends with it.
        cx.spawn_in(window, async move |this, cx| {
            let version = cx
                .background_spawn(async move { Arc::new(ObjectVersion::new(object, &render)) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                if view.live.load_seq == seq {
                    view.loaded(version, window, cx);
                }
            });
        })
        .detach();
    }

    /// Stops following the live object: the watch closes, work in flight is dropped. The tab
    /// closing calls it.
    pub(in crate::view) fn release_live(&mut self) {
        let live = &mut self.live;
        live.watch = None;
        live.watching = false;
        live.loading = None;
        live.merging = None;
        live.panel.task = None;
        live.pending = None;
        live.load_seq += 1;
    }

    /// The object this editor follows, once known.
    pub fn live_target(&self) -> Option<&ResourceRef> {
        self.live.target.as_ref()
    }

    /// The base / live / buffer state against the server.
    pub fn live_sync(&self) -> &LiveSync {
        &self.live.sync
    }

    /// The buffer now holds `version`: clean, followed.
    pub(super) fn loaded(
        &mut self,
        version: Arc<ObjectVersion>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = reference(&version.resource, self.live_cluster());
        self.set_buffer(&version.text, true, window, cx);
        self.live.sync.loaded(version);
        self.live.problem = None;
        if let Some(target) = target {
            self.bind(target, window, cx);
        }
        self.live.generation += 1;
        self.schedule_diff(Duration::ZERO, window, cx);
        cx.notify();
    }

    /// Replaces the buffer with `text`. A clean load moves the clean mark to it (the tab's dot
    /// goes) and keeps the cursor where it was; a merge leaves the buffer dirty.
    pub(super) fn set_buffer(
        &mut self,
        text: &str,
        clean: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let version = self.editor.update(cx, |editor, cx| {
            let mut api = editor.api(window, cx);
            let cursor = api.selections().first().map_or(0, |s| s.start);
            // The dimmed `status` of an opened object (E10-S06) points into the old text.
            api.clear_decorations();
            api.set_text(text);
            let mut at = cursor.min(text.len());
            while !text.is_char_boundary(at) {
                at -= 1;
            }
            api.select(at..at);
            api.version()
        });
        if clean {
            self.clean_version = version;
        }
        cx.emit(ItemEvent::UpdateTab);
    }

    /// Follows `target`: a watch on it replaces any earlier one. Following the same object again
    /// only restarts a watch that ended (the stream closed, or the cluster was not connected).
    pub(super) fn bind(
        &mut self,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.live.target.as_ref() == Some(&target) && self.live.watching {
            return;
        }
        self.live.target = Some(target.clone());
        self.live.watch = None;
        self.live.watching = false;
        let Some(reader) = self.live_reader(cx) else {
            return;
        };
        let (tx, mut rx) = futures::channel::mpsc::channel::<Watched>(WATCH_QUEUE);
        let producer = oxikube_runtime::spawn_kube(cx, watch_object(reader, target, tx));
        self.live.watching = true;
        self.live.watch = Some(cx.spawn_in(window, async move |this, cx| {
            // Dropped with this task (the editor closed, or bound elsewhere): the watch closes.
            let _producer = producer;
            while let Some(mut latest) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    latest = newer;
                }
                let taken = this.update_in(cx, |view, window, cx| {
                    view.live_arrived(latest, window, cx);
                });
                if taken.is_err() {
                    return;
                }
            }
            // The stream ended: the next diff or reload asks for a new watch. A flag, not the
            // task: a task never drops itself.
            let _ = this.update(cx, |view, _| view.live.watching = false);
        }));
    }

    /// The session's resource port, or why there is none (shown in the panel).
    pub(super) fn live_reader(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Arc<dyn ResourceReader>> {
        let reader = match &self.live.source {
            None => Err("No cluster: open the editor from a cluster tab to compare with it"),
            Some(_) if oxikube_runtime::mode(cx).is_none() => Err("The cluster is not connected"),
            Some(source) => source.reader(cx).ok_or("The cluster is not connected"),
        };
        match reader {
            Ok(reader) => Some(reader),
            Err(problem) => {
                self.show_problem(problem.into(), cx);
                None
            }
        }
    }

    fn live_cluster(&self) -> Option<ClusterId> {
        self.live.source.as_ref().map(|s| s.cluster().clone())
    }

    pub(super) fn show_problem(&mut self, problem: SharedString, cx: &mut Context<Self>) {
        self.live.panel.body = PanelBody::Message(problem.clone());
        self.live.problem = Some(problem);
        cx.notify();
    }

    /// A watch result: built off the UI thread (one build at a time; the latest result waits).
    fn live_arrived(&mut self, watched: Watched, window: &mut Window, cx: &mut Context<Self>) {
        self.live.pending = Some(watched);
        if !self.live.building {
            self.build_next(window, cx);
        }
    }

    fn build_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(watched) = self.live.pending.take() else {
            self.live.building = false;
            return;
        };
        self.live.building = true;
        let render = self.live.render.clone();
        // Detached with a flag, not stored: the task starts the next build itself, and a stored
        // task must not replace itself. It holds the editor weakly and ends with it.
        cx.spawn_in(window, async move |this, cx| {
            let built = cx
                .background_spawn(async move {
                    match watched {
                        Watched::Object(object) => Ok(LiveObject::Present(Arc::new(
                            ObjectVersion::new(object, &render),
                        ))),
                        Watched::Gone => Ok(LiveObject::Gone),
                        Watched::Failed(problem) => Err(problem),
                    }
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.apply_live(built, window, cx);
                view.build_next(window, cx);
            });
        })
        .detach();
    }

    fn apply_live(
        &mut self,
        built: Result<LiveObject, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let live = match built {
            Ok(live) => live,
            Err(problem) => {
                tracing::debug!("manifest editor: the live object's watch failed");
                self.show_problem(problem.into(), cx);
                return;
            }
        };
        self.live.problem = None;
        self.live.generation += 1;
        let dirty = self.is_dirty(cx);
        match self.live.sync.live_changed(live, dirty) {
            SyncAction::Reload(text) => self.set_buffer(&text, true, window, cx),
            SyncAction::Banner | SyncAction::None => {}
        }
        self.schedule_diff(Duration::ZERO, window, cx);
        notify_coalesced(cx);
    }

    /// The buffer changed (called for every edit): the sync state follows, the open diff is
    /// computed again after [`DIFF_DEBOUNCE`].
    pub(in crate::view) fn live_buffer_changed(
        &mut self,
        version: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.live.sync.state();
        self.live.sync.edited(version != self.clean_version);
        if before == SyncState::Merging && self.live.sync.state() != SyncState::Merging {
            self.live.panel.body = PanelBody::Loading;
        }
        self.schedule_diff(DIFF_DEBOUNCE, window, cx);
    }

    /// Finds the object the buffer's manifest names and either loads it (`load`) or only
    /// follows it (a diff of a buffer typed by hand). Off the UI thread; a later call cancels it.
    pub(super) fn find_from_buffer(
        &mut self,
        load: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(reader), Some(cluster)) = (self.live_reader(cx), self.live_cluster()) else {
            return;
        };
        let snapshot = self.editor.read(cx).snapshot(cx);
        self.live.loading = Some(cx.spawn_in(window, async move |this, cx| {
            let id = cx
                .background_spawn(async move {
                    document_for(&snapshot.to_text(), None).and_then(|doc| ObjectId::of(&doc))
                })
                .await;
            let id = match id {
                Ok(id) => id,
                Err(problem) => {
                    let _ = this.update(cx, |view, cx| {
                        view.show_problem(problem.to_string().into(), cx);
                    });
                    return;
                }
            };
            let found = oxikube_runtime::spawn_kube(cx, find_object(reader, id.clone())).await;
            let _ = this.update_in(cx, |view, window, cx| match found {
                Ok(Ok(Some(object))) if load => view.follow_object(object, None, window, cx),
                Ok(Ok(Some(object))) => {
                    if let Some(target) = reference(&object, Some(cluster)) {
                        view.bind(target, window, cx);
                    }
                }
                Ok(Ok(None)) if load => {
                    let problem = format!("There is no {} {} on the server", id.gvk.kind, id.name);
                    view.show_problem(problem.into(), cx);
                }
                Ok(Ok(None)) => {
                    let target = ResourceRef::new(cluster, id.gvk, id.namespace, id.name);
                    view.bind(target, window, cx);
                }
                Ok(Err(error)) => view.show_problem(error.to_string().into(), cx),
                Err(error) => view.show_problem(error.to_string().into(), cx),
            });
        }));
    }
}

/// The reference of `object` in `cluster`.
fn reference(object: &Resource, cluster: Option<ClusterId>) -> Option<ResourceRef> {
    Some(ResourceRef::new(
        cluster?,
        object.kind.clone(),
        object.namespace().map(Arc::from),
        object.name(),
    ))
}
