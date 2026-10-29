//! Uploading from the browser: the file dialog and the drop from the OS, the question before a
//! replacement, then `pod::UploadFile` through the bus.

use std::path::PathBuf;

use gpui::{AppContext as _, Context, PathPromptOptions, Window};
use oxikube_app::files::UploadPlan;
use oxikube_domain::OxiResult;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;
use oxikube_workspace::modal::DialogModal;

use super::FilesView;

impl FilesView {
    /// Asks which files to upload (the toolbar's button), then uploads them into the directory
    /// shown.
    pub fn upload_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Upload".into()),
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            this.update_in(cx, |view, window, cx| view.begin_upload(paths, window, cx))
                .ok();
        });
        self.tasks.push(task);
    }

    /// Uploads local `paths` into the directory shown: each is looked at first (a folder is
    /// refused, an existing file of the same name is asked about), then sent through
    /// `pod::UploadFile`. The toolbar's button and a drop from the OS both end here.
    pub fn begin_upload(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(container) = self.container.clone() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let (runner, pod, dest) = (
            self.deps.runner.clone(),
            self.pod.clone(),
            self.path.clone(),
        );
        let work = spawn_kube(cx, async move {
            let mut plans = Vec::with_capacity(paths.len());
            for path in paths {
                let plan = runner
                    .plan_upload(&pod, Some(&container), &dest, &path)
                    .await;
                plans.push((path, plan));
            }
            plans
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            let Ok(plans) = work.await else {
                return;
            };
            this.update_in(cx, |view, window, cx| {
                view.review_uploads(plans, window, cx)
            })
            .ok();
        });
        self.tasks.push(task);
    }

    fn review_uploads(
        &mut self,
        planned: Vec<(PathBuf, OxiResult<UploadPlan>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut plans = Vec::new();
        for (path, plan) in planned {
            match plan {
                Ok(plan) => plans.push(plan),
                Err(error) => {
                    let name = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.toast(
                        Toast::error(format!("{name}: {}", error.message()))
                            .key(format!("files/result/{name}")),
                        cx,
                    );
                }
            }
        }
        let clashes: Vec<String> = plans
            .iter()
            .filter(|plan| plan.exists)
            .map(|plan| plan.name.clone())
            .collect();
        if plans.is_empty() {
            return;
        }
        if clashes.is_empty() {
            self.run_uploads(plans, cx);
            return;
        }
        // Replacing a file is a separate question: ask before anything is sent.
        let Some(workspace) = self.workspace.as_ref().and_then(|ws| ws.upgrade()) else {
            return;
        };
        let title = if clashes.len() == 1 {
            format!("Replace {} in {}?", clashes[0], self.path)
        } else {
            format!("Replace {} files in {}?", clashes.len(), self.path)
        };
        let message = format!(
            "{} already {} in the container and would be overwritten.",
            clashes.join(", "),
            if clashes.len() == 1 {
                "exists"
            } else {
                "exist"
            }
        );
        let view = cx.entity().downgrade();
        workspace.update(cx, |workspace, cx| {
            let dialog = cx.new(|cx| {
                DialogModal::new(title, cx)
                    .message(message)
                    .confirm_label("Replace")
                    .destructive()
                    .on_confirm(move |_, cx| {
                        let plans = plans.clone();
                        view.update(cx, |view, cx| view.run_uploads(plans, cx)).ok();
                    })
            });
            workspace.show_modal(dialog, window, cx);
        });
    }

    /// Uploads the planned files one after the other; a file that exists is replaced (the user
    /// agreed, or there was none to replace).
    fn run_uploads(&mut self, plans: Vec<UploadPlan>, cx: &mut Context<Self>) {
        let Some(container) = self.container.clone() else {
            return;
        };
        let (runner, pod, dest) = (
            self.deps.runner.clone(),
            self.pod.clone(),
            self.path.clone(),
        );
        let work = spawn_kube(cx, async move {
            let mut done = Vec::with_capacity(plans.len());
            for plan in plans {
                let result = runner
                    .upload(&pod, Some(&container), &dest, &plan, plan.exists)
                    .await;
                done.push((plan.name, result));
            }
            done
        });
        self.watch_transfers(cx);
        let task = cx.spawn(async move |this, cx| {
            let Ok(done) = work.await else {
                return;
            };
            this.update(cx, |view, cx| {
                let uploaded = done.iter().any(|(_, result)| result.is_ok());
                for (name, result) in done {
                    view.transfer_finished(&name, result, cx);
                }
                // The directory has new files in it.
                if uploaded {
                    view.refresh(cx);
                }
            })
            .ok();
        });
        self.tasks.push(task);
    }
}
