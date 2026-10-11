//! An editor opened from a live object (E10-S06) follows that object too: the diff, the
//! changed-on-server banner and the watch work on `resource::Edit` tabs without a second read.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::Resource;

use crate::diff::{ObjectVersion, Renderer, buffer_text};
use crate::open::{RenderOptions, render_resource};
use crate::view::ManifestEditor;

impl ManifestEditor {
    /// The buffer shows `resource` rendered with `options`: that is the base the edits and the
    /// server's changes are compared with, and a reload or a merge writes the object the same
    /// way. Starts the watch on the object. The version is built off the UI thread; one that
    /// arrives after the object was shown again is dropped. Does nothing without a cluster
    /// to read from (an editor with no live source).
    pub(crate) fn follow_opened(
        &mut self,
        resource: Arc<Resource>,
        options: RenderOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.binding.as_ref().map(|b| b.target.clone()) else {
            return;
        };
        if !self.live.has_source() {
            return;
        }
        let render: Renderer = Arc::new(move |object: &Resource| {
            render_resource(object, options).map_or_else(|_| buffer_text(object), |m| m.text)
        });
        self.live.render = render.clone();
        self.live.load_seq += 1;
        let seq = self.live.load_seq;
        // Detached, not stored: a newer call makes this one stale (`load_seq`), and it holds
        // the editor weakly.
        cx.spawn_in(window, async move |this, cx| {
            let version = cx
                .background_spawn(async move {
                    Arc::new(ObjectVersion::new(Resource::clone(&resource), &render))
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                if view.live.load_seq != seq {
                    return;
                }
                view.live.sync.loaded(version);
                // Typed in the meantime (an edit tab unlocks with the object): not clean.
                let dirty = view.is_dirty(cx);
                view.live.sync.edited(dirty);
                view.live.problem = None;
                view.bind(target, window, cx);
                view.live.generation += 1;
                view.schedule_diff(Duration::ZERO, window, cx);
                cx.notify();
            });
        })
        .detach();
    }
}
