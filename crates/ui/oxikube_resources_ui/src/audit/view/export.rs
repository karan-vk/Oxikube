//! `audit::Export`: asks where, then writes what the filters match through the `FsPort`, a page
//! at a time, off the UI thread. Cancelling the save panel writes nothing.

use std::path::PathBuf;

use gpui::Context;
use oxikube_app::audit::export::{export, suggested_file_name};
use oxikube_domain::audit::AuditExportFormat;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::AuditView;

impl AuditView {
    /// Starts an export of the filtered records as `format`.
    pub fn export(&mut self, format: AuditExportFormat, cx: &mut Context<Self>) {
        let name = suggested_file_name(format, self.deps.clock.now());
        let answer = cx.prompt_for_new_path(&home_directory(), Some(&name));
        // Replaced (and so cancelled) by the next export, never cleared from inside.
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            this.update(cx, |view, cx| view.write_export(path, format, cx))
                .ok();
        }));
    }

    /// Writes the filtered records to `path`: read, formatted and written on the Tokio bridge.
    pub(crate) fn write_export(
        &mut self,
        path: PathBuf,
        format: AuditExportFormat,
        cx: &mut Context<Self>,
    ) {
        let (fs, reader, filters, now) = (
            self.deps.fs.clone(),
            self.deps.reader.clone(),
            self.filters.clone(),
            self.deps.clock.now(),
        );
        let target = path.clone();
        let work = spawn_kube(cx, async move {
            export(&*fs, &target, reader, filters, now, format).await
        });
        self.export_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |view, cx| {
                let toast = match result {
                    Ok(Ok(summary)) => Toast::success(format!(
                        "Exported {} audit records to {}",
                        summary.records,
                        path.display()
                    )),
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "exporting the audit log failed");
                        Toast::error(format!(
                            "Could not export the audit log: {}",
                            error.message()
                        ))
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the audit export stopped");
                        Toast::error("The export stopped before it finished.")
                    }
                };
                view.toast(toast.key("audit-export"), cx);
            })
            .ok();
        }));
    }
}

/// Where the save panel opens: the user's home.
fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
