//! Downloading from the browser: the save dialog, then `pod::DownloadFile` through the bus.

use std::path::PathBuf;

use gpui::{Context, PathPromptOptions};
use oxikube_app::files::{FileEntry, FileKind, is_secret_mount, join};
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::FilesView;

impl FilesView {
    /// Where a save dialog starts: the user's Downloads folder (the dialog falls back by itself
    /// when there is none; nothing here touches the disk).
    fn downloads_dir() -> PathBuf {
        std::env::var_os("HOME").map_or_else(
            || PathBuf::from("."),
            |home| PathBuf::from(home).join("Downloads"),
        )
    }

    /// Downloads the selected entry (the toolbar's button): asks where to put it.
    pub fn download_selected(&mut self, as_archive: bool, cx: &mut Context<Self>) {
        if let Some(entry) = self.selected_entry(cx) {
            self.download_entry(entry, as_archive, cx);
        }
    }

    /// Asks where to save `entry` (a file name for a file or a `.tar`, a folder for a directory
    /// that is unpacked), then downloads it.
    pub fn download_entry(&mut self, entry: FileEntry, as_archive: bool, cx: &mut Context<Self>) {
        let directory = entry.kind == FileKind::Directory;
        if is_secret_mount(&join(&self.path, &entry.name)) {
            self.toast(
                Toast::warning(
                    "This path holds mounted secrets: the file may contain credentials.",
                )
                .key("files/secret-mount"),
                cx,
            );
        }
        let start = Self::downloads_dir();
        let task = if directory && !as_archive {
            let chosen = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Download here".into()),
            });
            cx.spawn(async move |this, cx| {
                let Ok(Ok(Some(paths))) = chosen.await else {
                    return;
                };
                if let Some(folder) = paths.into_iter().next() {
                    this.update(cx, |view, cx| view.start_download(entry, folder, false, cx))
                        .ok();
                }
            })
        } else {
            let suggested = if as_archive {
                format!("{}.tar", entry.name)
            } else {
                entry.name.clone()
            };
            let chosen = cx.prompt_for_new_path(&start, Some(&suggested));
            cx.spawn(async move |this, cx| {
                let Ok(Ok(Some(path))) = chosen.await else {
                    return;
                };
                this.update(cx, |view, cx| {
                    view.start_download(entry, path, as_archive, cx)
                })
                .ok();
            })
        };
        self.tasks.push(task);
    }

    /// Downloads `entry` to `destination` (a file, or the folder a directory is unpacked into),
    /// through `pod::DownloadFile`.
    pub fn start_download(
        &mut self,
        entry: FileEntry,
        destination: PathBuf,
        as_archive: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(container) = self.container.clone() else {
            return;
        };
        let command = Command::PodDownloadFile {
            target: self.pod.clone(),
            container: Some(container.to_string()),
            path: join(&self.path, &entry.name),
            directory: entry.kind == FileKind::Directory,
            destination: destination.to_string_lossy().into_owned(),
            as_archive,
            size: entry.size,
        };
        let runner = self.deps.runner.clone();
        let work = spawn_kube(cx, async move { runner.download(command).await });
        self.jobs += 1;
        self.watch_transfers(cx);
        let name = entry.name;
        let task = cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or_else(|error| Err(error.into()));
            this.update(cx, |view, cx| {
                view.jobs = view.jobs.saturating_sub(1);
                view.transfer_finished(&name, result, cx);
            })
            .ok();
        });
        self.tasks.push(task);
    }

    /// Shows how a transfer ended.
    pub(super) fn transfer_finished(
        &mut self,
        name: &str,
        result: OxiResult<String>,
        cx: &mut Context<Self>,
    ) {
        let key = format!("files/result/{name}");
        let toast = match result {
            Ok(message) => Toast::success(message),
            Err(error) if error.message().contains("cancelled") => {
                Toast::info(format!("Cancelled: {name}"))
            }
            Err(error) => Toast::error(format!("{name}: {}", error.message())).persistent(),
        };
        self.toast(toast.key(key), cx);
        self.tick_transfers(cx);
        // A finished upload changed the directory; a finished download did not.
        cx.notify();
    }
}
