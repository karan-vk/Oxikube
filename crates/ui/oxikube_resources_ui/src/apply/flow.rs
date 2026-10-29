//! What the panel does: load, plan, dry-run, apply, cancel, re-run, open in the editor.
//!
//! All work is async and off the UI thread: reading files and building with kustomize, ordering
//! the plan, and the dry run and apply streams run on the Kubernetes runtime
//! (`oxikube_runtime::spawn_kube`, aborted when the work is replaced or the panel closes). The
//! results come back over a channel and are folded into the model as they arrive; the redraw is
//! coalesced to frame cadence.

use std::path::PathBuf;
use std::time::Instant;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::Context;
use oxikube_app::apply::ApplyError;
use oxikube_runtime::{notify_coalesced, spawn_kube};

use super::backend::{Loaded, ResultStream};
use super::model::{Activity, LoadSummary, Stage};
use super::source::{ApplySource, SourceKind};
use super::view::ApplyView;

/// Why a dry run that is stopped says its objects were not rehearsed.
const CANCELLED: &str = "cancelled";

impl ApplyView {
    /// Loads `source`: reads it, orders the plan and, unless the cluster is read-only, starts the
    /// server's dry run of every object.
    pub fn load(&mut self, source: ApplySource, cx: &mut Context<Self>) {
        self.load_keeping(source, None, cx);
    }

    fn load_keeping(
        &mut self,
        source: ApplySource,
        keep: Option<Vec<(String, String)>>,
        cx: &mut Context<Self>,
    ) {
        self.stop_work();
        self.read_only = self.deps.backend.read_only(&self.deps.cluster);
        self.model.begin_load();
        self.problem = None;
        self.confirming = false;
        self.plan = None;
        self.docs.clear();
        self.source = Some(source.clone());
        let load = self.deps.backend.load(source.clone());
        let remember = (source.kind == SourceKind::Manifests).then(|| {
            let directory = remembered_directory(&source);
            self.deps
                .backend
                .remember_directory(self.deps.cluster.clone(), directory)
        });
        self.work = Some(cx.spawn(async move |this, cx| {
            let loaded = spawn_kube(cx, load).await;
            if let Some(remember) = remember {
                let _ = spawn_kube(cx, remember).await;
            }
            this.update(cx, |this, cx| {
                match loaded {
                    Ok(Ok(loaded)) => this.loaded(loaded, keep, cx),
                    Ok(Err(error)) => this.load_failed(error.to_string(), cx),
                    Err(error) => this.load_failed(error.to_string(), cx),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn load_failed(&mut self, message: String, _: &mut Context<Self>) {
        self.model.set_activity(Activity::Idle);
        self.problem = Some(message);
    }

    fn loaded(
        &mut self,
        loaded: Loaded,
        keep: Option<Vec<(String, String)>>,
        cx: &mut Context<Self>,
    ) {
        let mut docs = loaded.docs;
        if let Some(keep) = keep {
            docs.retain(|doc| {
                let text = |pointer: &str| {
                    doc.json
                        .pointer(pointer)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                };
                keep.iter()
                    .any(|(kind, name)| kind == text("/kind") && name == text("/metadata/name"))
            });
        }
        self.model.loaded(LoadSummary {
            files: loaded.files,
            documents: docs.len(),
            errors: loaded.errors,
            notes: loaded.notes,
            tool: loaded.tool.map(|tool| tool.to_string()),
            stderr: loaded.stderr,
        });
        self.docs = docs;
        self.plan_documents(cx);
    }

    /// Orders the loaded documents for the namespace in the field, then dry-runs them.
    fn plan_documents(&mut self, cx: &mut Context<Self>) {
        if self.docs.is_empty() {
            self.model.set_activity(Activity::Idle);
            return;
        }
        let plan = self.deps.backend.plan(self.docs.clone(), self.target(cx));
        self.model.set_activity(Activity::Loading);
        self.work = Some(cx.spawn(async move |this, cx| {
            let plan = spawn_kube(cx, plan).await;
            this.update(cx, |this, cx| {
                match plan {
                    Ok(plan) => {
                        this.model.planned(&plan.plan);
                        this.plan = Some(plan);
                        this.start_dry_run(cx);
                    }
                    Err(error) => this.load_failed(error.to_string(), cx),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Orders the documents again under the namespace now in the field (Enter in the field).
    pub fn replan(&mut self, cx: &mut Context<Self>) {
        if self.model.busy() || self.docs.is_empty() {
            return;
        }
        self.stop_work();
        self.confirming = false;
        self.problem = None;
        self.plan_documents(cx);
        cx.notify();
    }

    /// The server's dry run of every object; skipped on a read-only cluster, whose guard
    /// refuses it, with the reason shown instead.
    fn start_dry_run(&mut self, cx: &mut Context<Self>) {
        self.read_only = self.deps.backend.read_only(&self.deps.cluster);
        let Some(plan) = self.plan.clone() else {
            return;
        };
        if self.read_only {
            self.model.set_activity(Activity::Idle);
            return;
        }
        self.model.begin_dry_run();
        let options = self.options();
        let stream = self.deps.backend.dry_run(plan, options);
        self.consume(stream, cx);
    }

    /// Re-runs the dry run of the plan in hand.
    pub fn dry_run(&mut self, cx: &mut Context<Self>) {
        if self.model.busy() || self.plan.is_none() {
            return;
        }
        self.stop_work();
        self.confirming = false;
        self.problem = None;
        self.start_dry_run(cx);
        cx.notify();
    }

    /// The Apply button: asks for the confirmation, which shows what the dry run said.
    pub fn request_apply(&mut self, cx: &mut Context<Self>) {
        if self.apply_blocked().is_some() || self.confirming {
            return;
        }
        self.confirming = true;
        cx.notify();
    }

    /// The user declined the confirmation.
    pub fn decline_apply(&mut self, cx: &mut Context<Self>) {
        if self.confirming {
            self.confirming = false;
            cx.notify();
        }
    }

    /// The user confirmed: one guarded `resource::ApplyPlan` dispatch, its results streamed in.
    pub fn confirm_apply(&mut self, cx: &mut Context<Self>) {
        if !self.confirming {
            return;
        }
        self.confirming = false;
        self.read_only = self.deps.backend.read_only(&self.deps.cluster);
        let Some(plan) = self.plan.clone().filter(|_| !self.read_only) else {
            cx.notify();
            return;
        };
        self.stop_work();
        self.problem = None;
        self.model.begin_apply();
        let options = self.options();
        let stream = self.deps.backend.apply(plan, options);
        self.consume(stream, cx);
        cx.notify();
    }

    /// Cancels the work in flight. A load or a dry run stops at once; an apply stops between
    /// objects and reports the rest skipped.
    pub fn cancel_run(&mut self, cx: &mut Context<Self>) {
        match self.model.activity() {
            Activity::Idle => {}
            Activity::Applying => {
                self.cancel.cancel();
            }
            Activity::Loading | Activity::DryRunning => {
                self.stop_work();
                self.model.skip_unanswered(CANCELLED);
                self.model.set_activity(Activity::Idle);
            }
        }
        self.confirming = false;
        cx.notify();
    }

    /// Re-plans from disk: reads the source again (after a fix) and dry-runs the result.
    pub fn rerun(&mut self, cx: &mut Context<Self>) {
        if self.model.busy() {
            return;
        }
        if let Some(source) = self.source.clone() {
            self.load_keeping(source, None, cx);
        }
    }

    /// Like [`rerun`](Self::rerun), keeping only the objects that failed or were skipped.
    pub fn rerun_failed(&mut self, cx: &mut Context<Self>) {
        if self.model.busy() {
            return;
        }
        let keep = self.model.failed_keys();
        if keep.is_empty() {
            return;
        }
        if let Some(source) = self.source.clone() {
            self.load_keeping(source, Some(keep), cx);
        }
    }

    fn stop_work(&mut self) {
        self.cancel.cancel();
        self.work = None;
        self.started = None;
    }

    /// Folds `stream` into the model as the results arrive.
    fn consume(&mut self, mut stream: ResultStream, cx: &mut Context<Self>) {
        let (sender, mut results) = mpsc::unbounded();
        self.started = Some(Instant::now());
        self.work = Some(cx.spawn(async move |this, cx| {
            // Owned by this task: dropped, and so aborted, with it.
            let pump = spawn_kube(cx, async move {
                while let Some(item) = stream.next().await {
                    if sender.unbounded_send(item).is_err() {
                        break;
                    }
                }
            });
            while let Some(item) = results.next().await {
                if this
                    .update(cx, |this, cx| this.on_result(item, cx))
                    .is_err()
                {
                    return;
                }
            }
            let _ = pump.await;
            this.update(cx, |this, cx| this.run_done(cx)).ok();
        }));
    }

    fn on_result(
        &mut self,
        item: Result<oxikube_app::apply::ObjectResult, ApplyError>,
        cx: &mut Context<Self>,
    ) {
        match item {
            Ok(result) => {
                let elapsed = self.started.map(|at| at.elapsed()).unwrap_or_default();
                self.model.record(result, elapsed);
            }
            Err(error) => {
                self.model.skip_unanswered("nothing was sent");
                self.problem = Some(error.to_string());
            }
        }
        notify_coalesced(cx);
    }

    fn run_done(&mut self, cx: &mut Context<Self>) {
        self.model.set_activity(Activity::Idle);
        self.started = None;
        cx.notify();
    }

    /// Whether the Load view has anything to show (a load ran).
    pub fn has_loaded(&self) -> bool {
        self.model.summary().is_some() || self.problem.is_some()
    }

    /// Shows the Load, Review or Result view when it has something to show.
    pub fn show_stage(&mut self, stage: Stage, cx: &mut Context<Self>) {
        let possible = match stage {
            Stage::Load => true,
            Stage::Review | Stage::Result => !self.model.rows().is_empty(),
        };
        if possible {
            self.model.show(stage);
            cx.notify();
        }
    }
}

/// The directory to remember for `source`: the directory itself, or a file's parent.
fn remembered_directory(source: &ApplySource) -> PathBuf {
    if source.path.extension().is_some() {
        source
            .path
            .parent()
            .map_or_else(|| source.path.clone(), PathBuf::from)
    } else {
        source.path.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_remembers_its_directory_and_a_directory_itself() {
        assert_eq!(
            remembered_directory(&ApplySource::manifests("/work/app/a.yaml")),
            PathBuf::from("/work/app")
        );
        assert_eq!(
            remembered_directory(&ApplySource::manifests("/work/app")),
            PathBuf::from("/work/app")
        );
    }
}
