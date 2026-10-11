//! The validation pipeline of a [`ManifestEditor`] (E10-S04, made cancellable and progressive in
//! E10-S11).
//!
//! 1. **Debounce.** Every change cancels the running pass (its [`CancelToken`]) and replaces its
//!    task with a new one that waits [`VALIDATION_DEBOUNCE`](super::VALIDATION_DEBOUNCE): twenty
//!    keystrokes in a row start one pass, after the last.
//! 2. **Parse** on the background executor, from a snapshot of the rope (taken on the UI thread
//!    in O(1)); a pass over a version already parsed (a schema arrived) reuses the cached parse.
//! 3. **Plan and check** a chunk at a time on the background executor (see
//!    [`super::progressive`]): the objects on screen first; after each chunk the findings so far
//!    are shown (one publish for a buffer under 1 MiB) and the kinds without a schema fetched.
//! 4. **Stale results are dropped**: a result is shown only for the buffer's current version, and
//!    a chunk polls the token, so a pass the buffer has outrun stops within a few hundred nodes.
//!
//! The task is kept in a field and replaced, never cleared from inside itself (gpui gotcha); the
//! pass marks itself finished through `pass_running` instead.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::OxiError;
use oxikube_domain::ids::Gvk;

use super::manifest_editor::ManifestEditor;
use super::model::Accepted;
use super::progressive::{Progress, check_chunk, plan};
use crate::cancel::{CancelOnDrop, CancelToken, Cancelled};
use crate::open::ResourceBinding;
use crate::yaml::parse_cancellable;

impl ManifestEditor {
    /// Validates the buffer after `delay`, cancelling the pass before it (see the module docs).
    pub(crate) fn schedule_validation(
        &mut self,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pass_cancel.cancel();
        if std::mem::take(&mut self.pass_running) {
            self.model.stats_mut().cancelled += 1;
        }
        if !self.settings(cx).validation {
            // Off (E10-S12): no squiggles, no work, no schema fetches.
            self.validation = None;
            let model = &mut self.model;
            self.editor.update(cx, |editor, cx| {
                model.clear(&mut editor.api(window, cx));
            });
            return;
        }
        let cancel = CancelToken::new();
        self.pass_cancel = CancelOnDrop::new(cancel.clone());
        self.validation = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((snapshot, cached, visible, known, mode, masking)) =
                this.update(cx, |view, cx| {
                    view.pass_running = true;
                    view.model.stats_mut().started += 1;
                    let editor = view.editor.read(cx);
                    let snapshot = editor.snapshot(cx);
                    let cached = view.model.parsed(snapshot.version());
                    let known = Arc::new(view.model.known().clone());
                    let masking = view.binding.as_ref().is_some_and(ResourceBinding::masking);
                    (
                        snapshot,
                        cached,
                        editor.visible_range(cx),
                        known,
                        view.model.mode(),
                        masking,
                    )
                })
            else {
                return;
            };
            let version = snapshot.version();
            let fresh = cached.is_none();
            let token = cancel.clone();
            let planned = cx
                .background_spawn(async move {
                    let parsed = match cached {
                        Some(parsed) => parsed,
                        None => Arc::new(parse_cancellable(snapshot.to_text(), &token)?),
                    };
                    Ok::<_, Cancelled>(plan(version, parsed, visible, mode, masking))
                })
                .await;
            let Ok(plan) = planned else {
                return;
            };
            let parsed = plan.parsed.clone();
            if fresh {
                let cache = parsed.clone();
                let cached = this.update(cx, |view, _| {
                    view.model.stats_mut().parses += 1;
                    view.model.cache_parse(version, cache);
                });
                if cached.is_err() {
                    return;
                }
            }
            let mut progress = Progress::start(&plan);
            let mut chunks = plan.chunks.into_iter();
            loop {
                let checking = !progress.is_complete();
                if checking {
                    let chunk = chunks.next().unwrap_or_default();
                    let (parsed, known, token, room) = (
                        parsed.clone(),
                        known.clone(),
                        cancel.clone(),
                        progress.room(),
                    );
                    let checked = cx
                        .background_spawn(async move {
                            check_chunk(&parsed, &chunk, &known, room, &token)
                        })
                        .await;
                    let Ok(findings) = checked else {
                        return;
                    };
                    progress.add(findings);
                }
                let complete = progress.is_complete();
                let mut validation = progress.validation();
                // `to_fetch` skips the kinds known or already being fetched.
                let missing = std::mem::take(&mut validation.missing);
                let shown = this.update_in(cx, |view, window, cx| {
                    let model = &mut view.model;
                    if checking {
                        model.stats_mut().chunks += 1;
                    }
                    let accepted = view.editor.update(cx, |editor, cx| {
                        model.accept(&mut editor.api(window, cx), validation)
                    });
                    for gvk in view.model.to_fetch(&missing) {
                        view.fetch_schema(gvk, window, cx);
                    }
                    if accepted == Accepted::Stale || complete {
                        view.pass_running = false;
                        if accepted == Accepted::Shown {
                            view.model.stats_mut().completed += 1;
                        }
                    }
                    cx.notify();
                    accepted
                });
                if complete || !matches!(shown, Ok(Accepted::Shown)) {
                    return;
                }
            }
        }));
    }

    /// Fetches `gvk`'s schema off the UI thread and validates again when it arrives.
    pub(super) fn fetch_schema(&mut self, gvk: Gvk, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.schemas.clone() else {
            // No cluster: syntax only.
            self.model
                .schema_arrived(gvk, Err(OxiError::not_found("no cluster")));
            return;
        };
        let (Some(port), Some(_)) = (source.port(cx), oxikube_runtime::mode(cx)) else {
            // Not connected yet: asked again on the next validation.
            self.model.forget_fetch(&gvk);
            return;
        };
        let cluster = source.cluster().clone();
        let wanted = gvk.clone();
        let fetch =
            oxikube_runtime::spawn_kube(
                cx,
                async move { port.schema_for(&cluster, &wanted).await },
            );
        self.schema_fetches
            .push(cx.spawn_in(window, async move |this, cx| {
                let outcome = fetch
                    .await
                    .unwrap_or_else(|error| Err(OxiError::internal(error.to_string())));
                let _ = this.update_in(cx, |view, window, cx| {
                    view.model.schema_arrived(gvk, outcome);
                    view.schedule_validation(Duration::ZERO, window, cx);
                });
            }));
    }
}
