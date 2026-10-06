//! What the screen's buttons do.
//!
//! Each one ends in a `kubeconfig::*` [`Command`] sent through the backend; the file picker, the
//! paste dialog and the removal confirmation only decide what goes into it.

use std::path::PathBuf;

use gpui::{Context, PathPromptOptions, SharedString, Window};
use oxikube_domain::command::{Command, KubeconfigSourceRef, NewKubeconfigSource};
use oxikube_ports::{UserSource, UserSourceKind};
use oxikube_workspace::modal::DialogModal;

use super::SourcesView;
use crate::sources::paste::PasteDialog;

/// What the platform picker should let the user choose.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Files,
    Folders,
}

impl SourcesView {
    /// Opens the platform file picker for kubeconfig files and adds what is chosen
    /// (`kubeconfig::AddSource`). The picker is asynchronous: the UI thread never waits for it.
    pub fn add_file(&mut self, cx: &mut Context<Self>) {
        self.pick(Pick::Files, cx);
    }

    /// Opens the platform picker for folders and adds what is chosen.
    pub fn add_folder(&mut self, cx: &mut Context<Self>) {
        self.pick(Pick::Folders, cx);
    }

    fn pick(&mut self, what: Pick, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: what == Pick::Files,
            directories: what == Pick::Folders,
            multiple: true,
            prompt: Some(
                match what {
                    Pick::Files => "Add kubeconfig",
                    Pick::Folders => "Add folder",
                }
                .into(),
            ),
        });
        // Replacing the slot drops an earlier picker task that is still waiting (a second click
        // while the first dialog is open); the task never clears the slot itself.
        self.picker = Some(cx.spawn(async move |this, cx| {
            match chosen.await {
                Ok(Ok(Some(paths))) => {
                    this.update(cx, |this, cx| this.add_picked(&paths, what, cx))
                        .ok();
                }
                // Cancelled.
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.say(format!("The file picker failed: {error}"), true, cx);
                    })
                    .ok();
                }
                // The picker went away without answering.
                Err(_) => {}
            }
        }));
    }

    fn add_picked(&mut self, paths: &[PathBuf], what: Pick, cx: &mut Context<Self>) {
        let commands = paths
            .iter()
            .map(|path| {
                let path = path.display().to_string();
                Command::KubeconfigAddSource {
                    source: match what {
                        Pick::Files => NewKubeconfigSource::File { path },
                        Pick::Folders => NewKubeconfigSource::Dir { path },
                    },
                }
            })
            .collect();
        self.run_commands(commands, cx);
    }

    /// Adds the default entry (`KUBECONFIG`, else `~/.kube/config`) back to the list.
    pub fn add_default(&mut self, cx: &mut Context<Self>) {
        self.run_commands(
            vec![Command::KubeconfigAddSource {
                source: NewKubeconfigSource::Default,
            }],
            cx,
        );
    }

    /// Reads every source again (`kubeconfig::Reload`).
    pub fn reload_all(&mut self, cx: &mut Context<Self>) {
        self.run_commands(vec![Command::KubeconfigReload], cx);
    }

    /// Opens the paste dialog in the workspace's modal layer.
    pub fn open_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.deps.workspace.clone() else {
            self.say(
                "Open this screen in a workspace to paste a kubeconfig.",
                true,
                cx,
            );
            return;
        };
        let backend = self.deps.backend.clone();
        let view = cx.entity().downgrade();
        let opened = workspace.update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, |window, cx| {
                PasteDialog::new(backend, view, window, cx)
            });
        });
        if opened.is_err() {
            self.say("The workspace is closing.", true, cx);
        }
    }

    /// Asks before removing the source in row `ix`, and removes it when confirmed
    /// (`kubeconfig::RemoveSource`). The text says whether a file is deleted.
    pub fn confirm_remove(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.model.row(ix) else {
            return;
        };
        let source = row.source.clone();
        let deletes = self.deps.backend.deletes_file(&source);
        let label = row.label.clone();
        let Some(workspace) = self.deps.workspace.clone() else {
            self.say(
                "Open this screen in a workspace to remove a source.",
                true,
                cx,
            );
            return;
        };
        let view = cx.entity().downgrade();
        let title: SharedString = format!("Remove {label}?").into();
        let message: SharedString = removal_text(&source, deletes).into();
        let opened = workspace.update(cx, |workspace, cx| {
            workspace.toggle_modal(window, cx, |_, cx| {
                DialogModal::new(title, cx)
                    .message(message)
                    .confirm_label(if deletes { "Delete" } else { "Remove" })
                    .destructive()
                    .on_confirm(move |_, cx| {
                        view.update(cx, |this, cx| this.remove(&source, cx)).ok();
                    })
            });
        });
        if opened.is_err() {
            self.say("The workspace is closing.", true, cx);
        }
    }

    /// Removes `source` without asking (`kubeconfig::RemoveSource`). The confirmation has
    /// happened by now.
    pub(super) fn remove(&mut self, source: &UserSource, cx: &mut Context<Self>) {
        let path = source
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let which = match source.kind {
            UserSourceKind::Default => KubeconfigSourceRef::Default,
            UserSourceKind::File => KubeconfigSourceRef::File { path },
            UserSourceKind::Dir => KubeconfigSourceRef::Dir { path },
        };
        self.run_commands(vec![Command::KubeconfigRemoveSource { source: which }], cx);
    }
}

/// The confirmation text: what removing the source does, and what it leaves alone.
pub fn removal_text(source: &UserSource, deletes_file: bool) -> String {
    match (source.kind, deletes_file) {
        (_, true) => "This kubeconfig was pasted into Oxikube, which stores it in its own folder. \
            Removing it deletes that file. Its clusters leave the catalog and cannot be \
            recovered unless you paste the kubeconfig again."
            .to_owned(),
        (UserSourceKind::Default, false) => "Oxikube stops reading KUBECONFIG and ~/.kube/config. \
            The files themselves are not touched, and you can add this entry back."
            .to_owned(),
        (UserSourceKind::Dir, false) => "Oxikube stops reading the files in this folder. The \
            folder and its files are not touched."
            .to_owned(),
        (UserSourceKind::File, false) => "Oxikube stops reading this file. The file itself is \
            not touched: it is yours, and it stays where it is."
            .to_owned(),
    }
}
