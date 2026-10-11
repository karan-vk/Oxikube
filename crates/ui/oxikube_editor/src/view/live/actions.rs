//! What the `editor::*` diff commands do to an editor (the bus handlers reach these through
//! [`EditorViews`](crate::view::EditorViews)), and the diff computed for the bottom panel.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, Context, Window};

use super::{DiffLayout, PanelBody};
use crate::diff::{LiveObject, ManifestDiff, MergeSide, MergeStep, ThreeWayMerge, document_for};
use crate::view::ManifestEditor;
use crate::view::apply::SecretEncoder;

impl ManifestEditor {
    /// Shows the diff against the live object in the bottom panel, or hides it
    /// (`editor::ShowDiff`). A buffer not following an object yet starts following the one its
    /// manifest names.
    pub fn toggle_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = &mut self.live.panel;
        panel.open = !panel.open;
        if !panel.open {
            panel.task = None;
            cx.notify();
            return;
        }
        panel.body = PanelBody::Loading;
        match self.live.target.clone() {
            None => self.find_from_buffer(false, window, cx),
            // Restarts a watch that ended (no-op while it runs).
            Some(target) => self.bind(target, window, cx),
        }
        self.schedule_diff(Duration::ZERO, window, cx);
        cx.notify();
    }

    /// Whether the diff panel is open.
    pub fn diff_open(&self) -> bool {
        self.live.panel.open
    }

    /// The diff's layout.
    pub fn diff_layout(&self) -> DiffLayout {
        self.live.panel.layout
    }

    /// The diff shown, once computed.
    pub fn diff(&self) -> Option<Arc<ManifestDiff>> {
        match &self.live.panel.body {
            PanelBody::Diff(diff) => Some(diff.clone()),
            PanelBody::Loading | PanelBody::Message(_) => None,
        }
    }

    /// Switches the diff between side by side and inline (`editor::ToggleDiffLayout`).
    pub fn toggle_diff_layout(&mut self, cx: &mut Context<Self>) {
        let panel = &mut self.live.panel;
        panel.layout = match panel.layout {
            DiffLayout::SideBySide => DiffLayout::Inline,
            DiffLayout::Inline => DiffLayout::SideBySide,
        };
        cx.notify();
    }

    /// Replaces the buffer with the live object (`editor::ReloadFromServer`, the banner's
    /// "Reload"). A buffer not following an object loads the one its manifest names.
    pub fn reload_from_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.live.sync.reload() {
            self.set_buffer(&text, true, window, cx);
            self.live.generation += 1;
            self.schedule_diff(Duration::ZERO, window, cx);
            cx.notify();
            return;
        }
        if let Some(LiveObject::Gone) = self.live.sync.live() {
            self.show_problem("The object was deleted on the server".into(), cx);
            return;
        }
        self.find_from_buffer(true, window, cx);
    }

    /// Merges the server's change into the buffer, three-way against the version it was loaded
    /// from (`editor::MergeFromServer`). The line merge runs off the UI thread; a merge without
    /// conflicts goes straight into the buffer, one with conflicts waits for a side each.
    pub fn merge_from_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((base, server)) = self.live.sync.merge_sides() else {
            return;
        };
        let snapshot = self.editor.read(cx).snapshot(cx);
        let version = snapshot.version();
        self.live.merging = Some(cx.spawn_in(window, async move |this, cx| {
            let merge = cx
                .background_spawn({
                    let server = server.clone();
                    async move { ThreeWayMerge::new(&base.text, &snapshot.to_text(), &server.text) }
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                // The buffer or the object moved on meanwhile (a status-only tick does not
                // count): the user asks again.
                if view.editor.read(cx).version() != version || !view.live.sync.is_current(&server)
                {
                    return;
                }
                if let MergeStep::Done(text) = view.live.sync.merged(merge, server) {
                    view.set_buffer(&text, false, window, cx);
                }
                cx.notify();
            });
        }));
    }

    /// Keeps the buffer as it is over the server's change (`editor::KeepMine`).
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.live.sync.keep_mine();
        cx.notify();
    }

    /// Settles conflict `conflict` of the pending merge with `side` (`editor::TakeMine`,
    /// `editor::TakeServer`).
    pub fn take_side(&mut self, conflict: usize, side: MergeSide, cx: &mut Context<Self>) {
        if self.live.sync.choose(conflict, side) {
            cx.notify();
        }
    }

    /// Puts the settled merge in the buffer (`editor::AcceptMerge`); nothing while a conflict has
    /// no side.
    pub fn accept_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.live.sync.accept() {
            self.set_buffer(&text, false, window, cx);
            cx.notify();
        }
    }

    /// Computes the diff for the open panel after `delay`, off the UI thread; a later call
    /// cancels this one, and a result for an older buffer or object is dropped.
    pub(super) fn schedule_diff(
        &mut self,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.live.panel.open {
            return;
        }
        let generation = self.live.generation;
        self.live.panel.task = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((snapshot, live, encoder)) = this.update(cx, |view, cx| {
                (
                    view.editor.read(cx).snapshot(cx),
                    view.live.sync.live().cloned(),
                    view.secret_encoder(),
                )
            }) else {
                return;
            };
            // Not known yet: the first watch result schedules the diff again.
            let Some(live) = live else {
                return;
            };
            let version = snapshot.version();
            let body = cx
                .background_spawn(
                    async move { compare(&snapshot.to_text(), &live, encoder.as_ref()) },
                )
                .await;
            let _ = this.update(cx, |view, cx| {
                let current =
                    view.editor.read(cx).version() == version && view.live.generation == generation;
                if current {
                    view.live.panel.body = body;
                    cx.notify();
                }
            });
        }));
    }
}

/// The panel's body for the buffer `text` against `live`. A Secret shown as plaintext is
/// compared as the server would get it (base64), like the live object: otherwise every key reads
/// as changed.
fn compare(text: &str, live: &LiveObject, encoder: Option<&SecretEncoder>) -> PanelBody {
    let live = match live {
        LiveObject::Present(version) => Some(&version.resource),
        LiveObject::Gone => None,
    };
    let encoded;
    let text = match encoder {
        Some(encoder) => match encoder.encode(text) {
            Ok(text) => {
                encoded = text;
                encoded.as_str()
            }
            Err(error) => return PanelBody::Message(error.message().to_owned().into()),
        },
        None => text,
    };
    match document_for(text, live) {
        Ok(mine) => {
            let live = live.map(|resource| resource.to_value());
            PanelBody::Diff(Arc::new(ManifestDiff::of_manifests(live.as_ref(), &mine)))
        }
        Err(problem) => PanelBody::Message(problem.to_string().into()),
    }
}
