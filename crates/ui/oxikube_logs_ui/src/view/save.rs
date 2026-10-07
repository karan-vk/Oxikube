//! Save (k9s `ctrl-s`, `logs::Save`): offer what would be written, ask where, write it through the
//! `FsPort` off the UI thread.
//!
//! Nothing is written without the user: [`offer_save`](LogView::offer_save) only opens the dialog,
//! and the platform's save panel (opened by [`save_chosen`](LogView::save_chosen)) picks the path;
//! cancelling either writes nothing. The write runs on the Tokio bridge
//! ([`spawn_kube`](oxikube_runtime::spawn_kube), abort-on-drop, so closing the tab or starting
//! another save stops it), reads the session in bounded chunks and hands them to
//! [`FsPort::write_stream`](oxikube_ports::FsPort::write_stream), so a buffer of millions of lines
//! costs a quarter of a megabyte at a time and the UI thread only receives progress. The log's
//! content never reaches the app's own logs: only counts and the error text of the port do.

use std::path::PathBuf;

use futures::StreamExt as _;
use futures::channel::mpsc::unbounded;
use gpui::{AppContext as _, Context, Task, Window};
use oxikube_app::logs::LogReader;
use oxikube_app::logs::export::{self, ExportSpec, truncation_note};
use oxikube_domain::OxiError;
use oxikube_domain::log::LogSaveScope;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::LogView;
use super::text::group;
use crate::export::{SaveDialog, SaveOffer, SaveRequest, suggested_file_name};

impl LogView {
    /// What saving each scope would write: the lines on screen and the whole buffer, fixed now.
    /// Counts are exact and immediate without a filter; with one the dialog counts off-thread.
    fn save_offers(&self) -> Option<Vec<SaveOffer>> {
        let reader = self.session.as_ref()?.reader();
        let format = self.export_format();
        let (first, next) = reader.read(|buffer, _| (buffer.first_seq(), buffer.next_seq()));
        let visible = self.viewport_seqs().unwrap_or(next..next);
        let offers = [
            (LogSaveScope::Visible, visible),
            (LogSaveScope::All, first..next),
        ];
        Some(
            offers
                .into_iter()
                .map(|(scope, seqs)| {
                    let spec = self.spec_for(seqs.clone(), format);
                    let lines = spec.filter.is_none().then(|| spec.count(&reader));
                    SaveOffer { scope, seqs, lines }
                })
                .collect(),
        )
    }

    /// Opens the save dialog for `scope` (`logs::Save`): it says which lines would be written and
    /// how many, with the timestamp and pod prefix toggles, and goes on only when the user
    /// chooses a file.
    pub fn offer_save(&mut self, scope: LogSaveScope, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.as_ref().and_then(|w| w.upgrade());
        let (Some(offers), Some(workspace), Some(session)) =
            (self.save_offers(), workspace, self.session.as_ref())
        else {
            self.toast(Toast::info("There is no log to save yet."), cx);
            return;
        };
        // Only lines dropped since a clear: the view shows no marker for what the user cleared.
        let note = session
            .read(|buffer, _| truncation_note(buffer.dropped_since_clear(), buffer.capacity()));
        let reader = session.reader();
        let view = cx.entity().downgrade();
        let format = self.export_format();
        let filter = self.active_filter();
        let counting: Vec<SaveOffer> = offers
            .iter()
            .filter(|offer| offer.lines.is_none())
            .cloned()
            .collect();
        let dialog =
            cx.new(|cx| SaveDialog::new(view, offers, scope, format, filter.clone(), note, cx));
        workspace.update(cx, |workspace, cx| {
            workspace.show_modal(dialog.clone(), window, cx);
        });
        // A filter makes counting a scan: do it on the background executor and tell the dialog.
        for offer in counting {
            let spec = ExportSpec::new(offer.seqs.clone(), format).with_filter(filter.clone());
            let reader = reader.clone();
            let dialog = dialog.downgrade();
            cx.spawn(async move |_, cx| {
                let lines = cx
                    .background_executor()
                    .spawn(async move { spec.count(&reader) })
                    .await;
                dialog
                    .update(cx, |dialog, cx| dialog.set_lines(offer.scope, lines, cx))
                    .ok();
            })
            .detach();
        }
    }

    /// The user chose to save `request` (the dialog's "Choose file…"): asks the platform where,
    /// then writes. Cancelling the panel writes nothing and leaves a save in progress alone.
    pub fn save_chosen(&mut self, request: SaveRequest, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let reader = session.reader();
        let name = suggested_file_name(
            &self.target.name,
            self.options.container.as_deref(),
            jiff::Timestamp::now(),
        );
        let answer = cx.prompt_for_new_path(&save_directory(), Some(&name));
        self.save_prompt = Some(cx.spawn(async move |this, cx| {
            // Cancelled, or the platform could not show the panel: nothing to write.
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            this.update(cx, |view, cx| view.start_write(path, reader, request, cx))
                .ok();
        }));
    }

    /// Stops the save in progress, if any, and says so in its toast: dropping the job aborts the
    /// write (and its temporary file goes), so its persistent "Saving…" toast would otherwise
    /// stay up for ever. Called when another save starts and when the tab closes.
    pub(crate) fn stop_save(&mut self, cx: &mut Context<Self>) {
        self.save_prompt = None;
        let Some(job) = self.save_job.take() else {
            return;
        };
        if !job.done {
            let toast = Toast::info(format!("Stopped saving {}.", job.file)).key(job.key);
            // The tab closes inside an update of its workspace, which the toast needs: later.
            // (Through the workspace, which outlives a view that is being dropped.)
            let workspace = self.workspace.clone();
            cx.defer(move |cx| {
                if let Some(workspace) = workspace {
                    workspace
                        .update(cx, |workspace, cx| workspace.show_toast(toast, cx))
                        .ok();
                }
            });
        }
    }

    /// Writes `request` to `path` off the UI thread, reporting progress in a keyed toast that the
    /// result then replaces.
    fn start_write(
        &mut self,
        path: PathBuf,
        reader: LogReader,
        request: SaveRequest,
        cx: &mut Context<Self>,
    ) {
        self.stop_save(cx);
        let fs = self.deps.fs.clone();
        let (total, spec) = (request.lines, request.spec);
        let key = format!("logs-save:{}", path.display());
        let file = file_name(&path);
        let (tx, mut progress) = unbounded();
        self.toast(
            Toast::info(format!("Saving {file}…"))
                .key(key.clone())
                .persistent(),
            cx,
        );
        let work = {
            let path = path.clone();
            spawn_kube(cx, async move {
                export::save(&*fs, &path, reader, spec, Some(tx)).await
            })
        };
        let task = cx.spawn({
            let (key, file) = (key.clone(), file.clone());
            async move |this, cx| {
                let mut last_reported = 0;
                while let Some(lines) = progress.next().await {
                    // Report in steps of about a twentieth, or every 50 000 lines when the total
                    // is not known: a toast per chunk would be noise.
                    let step = total.map_or(50_000, |total| (total / 20).max(1));
                    if lines < last_reported + step {
                        continue;
                    }
                    last_reported = lines;
                    let words = match total {
                        Some(total) if total > 0 => {
                            format!("Saving {file}… {}%", (lines * 100 / total).min(100))
                        }
                        _ => format!("Saving {file}… {} lines", group(lines)),
                    };
                    let key = key.clone();
                    if this
                        .update(cx, |view, cx| {
                            view.toast(Toast::info(words).key(key).persistent(), cx)
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                let result = match work.await {
                    Ok(result) => result,
                    Err(error) => Err(OxiError::from(error)),
                };
                this.update(cx, |view, cx| {
                    let toast = match result {
                        Ok(summary) => Toast::success(format!(
                            "Saved {} lines to {}",
                            group(summary.lines),
                            path.display()
                        )),
                        Err(error) => {
                            tracing::warn!(%error, "saving the log failed");
                            Toast::error(format!("Could not save the log: {}", error.message()))
                        }
                    };
                    view.toast(toast.key(key), cx);
                    if let Some(job) = view.save_job.as_mut() {
                        job.done = true;
                    }
                })
                .ok();
            }
        });
        self.save_job = Some(SaveJob {
            key,
            file,
            done: false,
            _task: task,
        });
    }
}

/// The write in progress: dropping it stops the write.
pub(crate) struct SaveJob {
    /// The key of its toast.
    key: String,
    /// The file's name, for the toast that says it stopped.
    file: String,
    /// Whether the result was already reported.
    done: bool,
    _task: Task<()>,
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Where the save panel opens: the user's home directory, else the current one.
fn save_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
