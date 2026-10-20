//! Submitting the start dialog: `portforward::Start` on the Tokio bridge, and what follows.

use gpui::{Context, DismissEvent};
use oxikube_app::StartedForward;
use oxikube_domain::OxiError;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{Toast, ToastAction};

use super::prefs::LastUsed;
use super::problem::StartProblem;
use super::view::{Stage, StartDialog};

impl StartDialog {
    /// Starts the forward: runs `portforward::Start` on the Tokio bridge. Does nothing while a
    /// field is invalid (the fields say why).
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.stage != Stage::Editing {
            return;
        }
        let input = self.input(cx);
        let Ok(spec) = self.form.build(&input) else {
            cx.notify();
            return;
        };
        self.stage = Stage::Starting;
        self.problem = None;
        cx.notify();
        let (starter, workspace) = (self.starter.clone(), self.workspace.clone());
        let label = spec.name.clone().unwrap_or_else(|| spec.default_name());
        let (bind, auto) = (input.bind.clone(), input.local_auto);
        // Detached: the start must finish even if the dialog goes away meanwhile (the forward is
        // listed either way); the task ends itself and clears nothing.
        cx.spawn(async move |this, cx| {
            let run = spawn_kube(cx, async move { starter.start(spec).await });
            let result = match run.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            if result.is_ok() {
                cx.update(|cx| LastUsed::set(bind, auto, cx));
            }
            let delivered = this.update(cx, |this, cx| this.finished(&result, &label, cx));
            if delivered.is_err() {
                let toast = match &result {
                    Ok(started) => success_toast(&label, started),
                    Err(error) => Toast::error(StartProblem::of(error).message),
                };
                workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn finished(
        &mut self,
        result: &Result<StartedForward, OxiError>,
        label: &str,
        cx: &mut Context<Self>,
    ) {
        self.stage = Stage::Editing;
        match result {
            Ok(started) => {
                let toast = success_toast(label, started);
                self.workspace
                    .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                    .ok();
                cx.emit(DismissEvent);
            }
            Err(error) => {
                self.problem = Some(StartProblem::of(error));
                cx.notify();
            }
        }
    }
}

/// The toast after a start: the URL, and "Open in browser" (the system browser, never an embedded
/// web view).
pub(super) fn success_toast(label: &str, started: &StartedForward) -> Toast {
    let Some(url) = started.url() else {
        return Toast::success(format!("Forwarding {label}: waiting for a ready pod"));
    };
    let open = url.clone();
    Toast::success(format!("Forwarding {label} on {url}")).action(ToastAction::new(
        "Open in browser",
        move |_, cx| {
            cx.open_url(&open);
        },
    ))
}
