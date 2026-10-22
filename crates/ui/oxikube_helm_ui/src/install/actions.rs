//! The tab's actions: preview, install (ask, confirm), decline, cancel, and the form's toggles.

use futures::StreamExt as _;
use gpui::{Context, Window};
use oxikube_runtime::{NotifyCoalescedExt as _, spawn_kube};

use oxikube_domain::helm::ReleaseRef;

use super::model::{HelmInstallEvent, Shown, Stage};
use super::view::HelmInstallView;

/// Output lines taken off the channel per redraw at most.
const LINES_PER_BATCH: usize = 512;

impl HelmInstallView {
    /// Ticks or clears "Create namespace" (`--create-namespace`).
    pub fn set_create_namespace(&mut self, on: bool, cx: &mut Context<Self>) {
        self.create_namespace = on;
        cx.notify();
    }

    /// Ticks or clears "Wait for resources" (`--wait`).
    pub fn set_wait(&mut self, on: bool, cx: &mut Context<Self>) {
        self.wait = on;
        cx.notify();
    }

    /// Picks the chart version (empty: the newest). The defaults are not re-read: the values the
    /// person edited stay.
    pub fn set_version(&mut self, version: &str, cx: &mut Context<Self>) {
        version.clone_into(&mut self.version);
        cx.notify();
    }

    /// Puts a namespace the cluster has in the namespace field.
    pub fn pick_namespace(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = name.to_owned();
        self.namespace
            .update(cx, |input, cx| input.set_value(name, window, cx));
        cx.notify();
    }

    /// Runs the preview: the guard's dry run of what the form and the values say (`helm
    /// template` and the server's dry run), off the UI thread. A preview in flight is replaced.
    pub fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.stage.is_editable() {
            return;
        }
        let form = self.form(cx);
        let values_version = self.values_version(cx);
        let spec = match form.spec(
            &self.deps.cluster,
            &self.chart,
            self.values(cx),
            self.namespaces.as_deref(),
        ) {
            Ok(spec) => spec,
            Err(_) => {
                // The form shows what is wrong next to each field.
                cx.notify();
                return;
            }
        };
        self.stage = Stage::Previewing;
        self.preview_error = None;
        let preview = self.deps.backend.preview(spec);
        self.preview_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = spawn_kube(cx, preview).await;
            this.update_in(cx, |this, window, cx| {
                this.stage = Stage::Editing;
                match result
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    Ok(preview) => {
                        oxikube_ui::editor::set_text(&this.manifest, &preview.manifest, window, cx);
                        this.shown = Some(Shown {
                            preview,
                            form,
                            values_version,
                        });
                    }
                    Err(why) => {
                        this.shown = None;
                        this.preview_error = Some(why);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Asks to install what the (fresh) preview showed: the guard answers with its confirmation,
    /// which the tab shows. Nothing is installed yet.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.fresh_preview(cx).cloned() else {
            return;
        };
        if !self.stage.is_editable() || !shown.preview.can_install() {
            return;
        }
        self.stage = Stage::Asking;
        let ask = self.deps.backend.ask(shown.preview);
        self.install_task = Some(cx.spawn(async move |this, cx| {
            let asked = spawn_kube(cx, ask).await;
            this.update(cx, |this, cx| {
                this.stage = match asked
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    Ok(request) => Stage::Confirming(request),
                    Err(why) => Stage::Failed(why),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Confirms the guard's request: the install runs, its output streams into the log (batched,
    /// one redraw per frame at most). On success the tab emits [`HelmInstallEvent::Installed`]
    /// with the new release.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let (Stage::Confirming(request), Some(shown)) = (&self.stage, &self.shown) else {
            return;
        };
        let spec = &shown.preview.spec;
        let release = ReleaseRef::new(spec.cluster.clone(), &spec.namespace, &spec.release);
        let start = self.deps.backend.start(&shown.preview, request);
        self.stage = Stage::Installing;
        self.installed = None;
        self.log.clear();
        let mut lines = start.lines.ready_chunks(LINES_PER_BATCH);
        self.lines_task = Some(cx.spawn(async move |this, cx| {
            while let Some(batch) = lines.next().await {
                let alive = this.update(cx, |this, cx| {
                    for line in batch {
                        this.log.push(line);
                    }
                    let last = this.log.len().saturating_sub(1);
                    this.log_scroll
                        .scroll_to_item(last, gpui::ScrollStrategy::Bottom);
                    cx.notify_coalesced();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
        let run = start.run;
        self.install_task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_kube(cx, run).await;
            this.update(cx, |this, cx| {
                this.stage = match result
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    Ok(message) => {
                        this.installed = Some(release.clone());
                        cx.emit(HelmInstallEvent::Installed(release));
                        Stage::Installed(message)
                    }
                    Err(why) => Stage::Failed(why),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Declines the guard's request: nothing is installed (audited as cancelled).
    pub fn decline(&mut self, cx: &mut Context<Self>) {
        let Stage::Confirming(request) = &self.stage else {
            return;
        };
        let decline = self.deps.backend.decline(request.clone());
        self.stage = Stage::Editing;
        self.install_task = Some(cx.spawn(async move |_, cx| {
            spawn_kube(cx, decline).await.ok();
        }));
        cx.notify();
    }

    /// Stops a running install: the dispatch is dropped, which kills `helm` (the guard audits it
    /// as cancelled). What `helm` created so far stays; the log says so.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Installing {
            return;
        }
        self.install_task = None;
        self.stage = Stage::Failed(
            "Cancelled: helm was stopped. Objects it created already stay in the cluster; \
             uninstall the release to remove them."
                .to_owned(),
        );
        cx.notify();
    }

    /// Back to the form after an install ended (to change something and try again).
    pub fn edit_again(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Installed(_) | Stage::Failed(_)) {
            self.stage = Stage::Editing;
            cx.notify();
        }
    }
}
