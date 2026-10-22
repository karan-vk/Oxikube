//! The tab's actions: pick a release or a chart, the toggles, preview, upgrade (ask, confirm),
//! decline and cancel.

use futures::StreamExt as _;
use gpui::{Context, Window};
use oxikube_domain::helm::ReleaseRef;
use oxikube_runtime::{NotifyCoalescedExt as _, spawn_kube};

use super::backend::ChartSource;
use super::model::{HelmUpgradeEvent, Pane, Shown, Stage};
use super::view::HelmUpgradeView;

/// Output lines taken off the channel per redraw at most.
const LINES_PER_BATCH: usize = 512;

impl HelmUpgradeView {
    /// Ticks or clears `--reuse-values`.
    pub fn set_reuse_values(&mut self, on: bool, cx: &mut Context<Self>) {
        self.reuse_values = on;
        cx.notify();
    }

    /// Ticks or clears "Wait for resources" (`--wait`).
    pub fn set_wait(&mut self, on: bool, cx: &mut Context<Self>) {
        self.wait = on;
        cx.notify();
    }

    /// Picks the chart version (empty: the newest) and reads that version's default values for
    /// the chart-defaults diff. The values the person edited stay.
    pub fn set_version(&mut self, version: &str, cx: &mut Context<Self>) {
        version.clone_into(&mut self.version);
        self.target_defaults = None;
        self.defaults_diff = None;
        self.load_defaults(cx);
        cx.notify();
    }

    /// Picks one of the charts the repositories offer under the release's chart name.
    pub fn pick_source(
        &mut self,
        source: &ChartSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reference = source.chart.reference();
        self.chart
            .update(cx, |input, cx| input.set_value(reference, window, cx));
        self.version.clear();
        self.target_defaults = None;
        self.defaults_diff = None;
        self.load_defaults(cx);
        cx.notify();
    }

    /// Picks the release to upgrade (from the picker): the tab becomes that release's.
    pub fn pick_release(
        &mut self,
        release: ReleaseRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.stage != Stage::Choosing {
            return;
        }
        self.target = Some(release);
        self.load_target(window, cx);
        cx.notify();
    }

    /// Shows `pane` in the right column.
    pub fn set_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.pane = pane;
        cx.notify();
    }

    /// Runs the preview: the guard's dry run of what the form and the values say (`helm
    /// template`, the server's dry run, the objects that change), off the UI thread. A preview
    /// in flight is replaced.
    pub fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else {
            return;
        };
        if !self.stage.is_editable() {
            return;
        }
        let form = self.form(cx);
        let values_version = self.values_version(cx);
        let Ok(spec) = form.spec(&target, &self.sources, self.values(cx)) else {
            // The form shows what is wrong next to each field.
            cx.notify();
            return;
        };
        self.stage = Stage::Previewing;
        self.preview_error = None;
        self.pane = Pane::Preview;
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

    /// Asks to upgrade what the (fresh) preview showed: the guard answers with its confirmation,
    /// which the tab shows. Nothing is upgraded yet.
    pub fn upgrade(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.fresh_preview(cx).cloned() else {
            return;
        };
        if !self.stage.is_editable() || !shown.preview.can_upgrade() {
            return;
        }
        self.stage = Stage::Asking;
        let ask = self.deps.backend.ask(shown.preview);
        self.upgrade_task = Some(cx.spawn(async move |this, cx| {
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

    /// Confirms the guard's request: the upgrade runs, its output streams into the log (batched,
    /// one redraw per frame at most). On success the tab emits [`HelmUpgradeEvent::Upgraded`].
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let (Stage::Confirming(request), Some(shown)) = (&self.stage, &self.shown) else {
            return;
        };
        let release = shown.preview.spec.release.clone();
        let start = self.deps.backend.start(&shown.preview, request);
        self.stage = Stage::Upgrading;
        self.upgraded = None;
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
        self.upgrade_task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_kube(cx, run).await;
            this.update(cx, |this, cx| {
                this.stage = match result
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                {
                    Ok(message) => {
                        this.upgraded = Some(release.clone());
                        // The release now runs the chart that was applied.
                        this.shown = None;
                        cx.emit(HelmUpgradeEvent::Upgraded(release));
                        Stage::Upgraded(message)
                    }
                    Err(why) => Stage::Failed(failure_text(&why)),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Declines the guard's request: nothing is upgraded (audited as cancelled).
    pub fn decline(&mut self, cx: &mut Context<Self>) {
        let Stage::Confirming(request) = &self.stage else {
            return;
        };
        let decline = self.deps.backend.decline(request.clone());
        self.stage = Stage::Editing;
        self.upgrade_task = Some(cx.spawn(async move |_, cx| {
            spawn_kube(cx, decline).await.ok();
        }));
        cx.notify();
    }

    /// Stops a running upgrade: the dispatch is dropped, which kills `helm` (the guard audits it
    /// as cancelled). What `helm` changed so far stays; the log says so.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Upgrading {
            return;
        }
        self.upgrade_task = None;
        self.stage = Stage::Failed(
            "Cancelled: helm was stopped. The release may be left in a pending or failed \
             revision: open its History to roll back."
                .to_owned(),
        );
        cx.notify();
    }

    /// Back to the form after an upgrade ended (to change something and try again).
    pub fn edit_again(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Upgraded(_) | Stage::Failed(_)) {
            self.stage = Stage::Editing;
            cx.notify();
        }
    }
}

/// `why` with the way out of a failed upgrade: Helm leaves a `failed` revision.
fn failure_text(why: &str) -> String {
    format!(
        "{why}\nA failed upgrade leaves a failed revision: open the release's History to roll back."
    )
}
