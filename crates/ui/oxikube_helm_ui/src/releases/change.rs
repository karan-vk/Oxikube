//! Rolling back and uninstalling from the detail (E16-S06): the history row's rollback with its
//! preview, and the header's uninstall with its keep-history option.
//!
//! Both only *ask*: they send `helm::Rollback` / `helm::Uninstall` on the bus, whose guard
//! blocks a read-only cluster, shows the confirmation (a simple one for the rollback; the release
//! name typed for the uninstall), runs `helm` and audits it. The view never calls `helm`, and it
//! does not assume the command worked: the release records change, the page lists again
//! ([`follow_records`](super::view)) and the detail follows its release to the new revision, or
//! closes when the release is gone.
//!
//! The rollback preview is read natively: the values of the revision to restore against the
//! values of the release now (both masked, as the Values tab shows them), diffed off the UI
//! thread.

use gpui::Context;
use oxikube_app::helm_ops::{DiffLine, values_diff};
use oxikube_domain::command::Command;
use oxikube_domain::helm::HelmFeature;
use oxikube_runtime::spawn_kube;

use super::view::{HelmReleasesView, describe};

/// What rolling back to the selected revision would change in the values.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Preview {
    /// No revision is selected.
    #[default]
    None,
    /// The two values are being read.
    Loading,
    /// The lines that differ (empty: the values are the same).
    Ready(Vec<DiffLine>),
    /// A read failed; the text is for the user.
    Failed(String),
}

impl HelmReleasesView {
    /// Whether the gate lets the detail offer `feature`.
    pub(super) fn offers(&self, feature: HelmFeature, cx: &Context<Self>) -> bool {
        self.deps.gate.read(cx).allows(feature)
    }

    /// Selects `revision` of the open release for rollback and reads what it would change. The
    /// latest revision, an unknown one and a gate without rollback select nothing.
    pub fn select_revision(&mut self, revision: u32, cx: &mut Context<Self>) {
        if !self.offers(HelmFeature::Rollback, cx) {
            return;
        }
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        if revision == detail.release.revision
            || !detail
                .history
                .revisions
                .iter()
                .any(|r| r.revision == revision)
        {
            return;
        }
        detail.history.selected = Some(revision);
        detail.history.preview = Preview::Loading;
        let (release, now) = (detail.release.release.clone(), detail.release.revision);
        let generation = detail.generation;
        let backend = self.deps.backend.clone();
        let (read_now, read_target) = (
            backend.values(&release, now, false),
            backend.values(&release, revision, false),
        );
        detail.history.preview_task = Some(cx.spawn(async move |this, cx| {
            let result = spawn_kube(cx, async move {
                let (now, target) = futures::join!(read_now, read_target);
                Ok::<_, oxikube_domain::OxiError>((now?, target?))
            })
            .await;
            let diff = match result {
                Ok(Ok((now, target))) => {
                    // The diff of two masked texts runs on the background executor.
                    Ok(cx
                        .background_executor()
                        .spawn(async move { values_diff(&now.text, &target.text) })
                        .await)
                }
                Ok(Err(error)) => Err(describe(&error)),
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| {
                this.preview_arrived(generation, revision, diff, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn preview_arrived(
        &mut self,
        generation: u64,
        revision: u32,
        diff: Result<Vec<DiffLine>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self
            .detail
            .as_mut()
            .filter(|d| d.generation == generation && d.history.selected == Some(revision))
        else {
            return;
        };
        detail.history.preview = match diff {
            Ok(lines) => Preview::Ready(lines),
            Err(why) => Preview::Failed(why),
        };
        cx.notify();
    }

    /// Clears the selected revision.
    pub fn clear_revision(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.as_mut() else {
            return;
        };
        if detail.history.selected.take().is_some() {
            detail.history.preview = Preview::None;
            detail.history.preview_task = None;
            cx.notify();
        }
    }

    /// Asks to roll the open release back to the selected revision: `helm::Rollback` on the bus.
    pub fn rollback_selected(&mut self, cx: &mut Context<Self>) {
        if !self.offers(HelmFeature::Rollback, cx) {
            return;
        }
        let Some(detail) = &self.detail else { return };
        let Some(revision) = detail.history.selected else {
            return;
        };
        let release = &detail.release.release;
        self.deps.dispatcher.dispatch(
            Command::HelmRollback {
                cluster: release.cluster.clone(),
                namespace: release.namespace.clone(),
                release: release.name.clone(),
                revision,
            },
            cx,
        );
    }

    /// Whether the header's uninstall keeps the release history.
    pub fn keeps_history(&self) -> bool {
        self.detail.as_ref().is_some_and(|d| d.keep_history)
    }

    /// Sets the header's keep-history option.
    pub fn set_keep_history(&mut self, keep: bool, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.as_mut()
            && detail.keep_history != keep
        {
            detail.keep_history = keep;
            cx.notify();
        }
    }

    /// Asks to uninstall the open release: `helm::Uninstall` on the bus, with the keep-history
    /// option.
    pub fn uninstall(&mut self, cx: &mut Context<Self>) {
        if !self.offers(HelmFeature::Uninstall, cx) {
            return;
        }
        let Some(detail) = &self.detail else { return };
        let release = &detail.release.release;
        self.deps.dispatcher.dispatch(
            Command::HelmUninstall {
                cluster: release.cluster.clone(),
                namespace: release.namespace.clone(),
                release: release.name.clone(),
                keep_history: detail.keep_history,
            },
            cx,
        );
    }

    /// The preview of the selected revision, with the revision.
    pub fn preview(&self) -> Option<(u32, &Preview)> {
        let history = &self.detail.as_ref()?.history;
        Some((history.selected?, &history.preview))
    }
}
