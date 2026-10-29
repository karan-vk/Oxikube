//! The handlers of `pod::DownloadFile` and `pod::UploadFile`, and what the guard says about the
//! upload: its confirmation text and its audit detail.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command, CommandId};

use super::download::DownloadRequest;
use super::service::FileService;
use super::upload::UploadRequest;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};

/// Registers `pod::DownloadFile` and `pod::UploadFile` on `registry`, over the app's one
/// [`FileService`]. (`pod::BrowseFiles` opens a view: `oxikube_resources_ui::files` handles it.)
///
/// Both run to the end of the transfer, so call them from the Tokio bridge
/// (`oxikube_runtime::spawn_kube`); progress is on the service's board meanwhile. The download is
/// a read; the upload gets the guard's [`Mutation`](crate::Mutation) (read-only check,
/// confirmation, audit) before its handler runs.
///
/// # Errors
///
/// A [`RegisterError`] when either command is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    service: Arc<FileService>,
) -> Result<(), RegisterError> {
    let download = *command::lookup(CommandId::POD_DOWNLOAD_FILE)
        .ok_or(RegisterError::Undeclared(CommandId::POD_DOWNLOAD_FILE))?;
    let for_download = service.clone();
    registry.register(download, move |command: Command, cx: HandlerContext| {
        let service = for_download.clone();
        async move {
            // A download writes a path on the user's disk, and the command is not confirmed: only
            // the app's own UI and the palette may start one, never an agent or a plugin.
            if !matches!(cx.initiator(), Initiator::Ui | Initiator::Command) {
                return Err(OxiError::forbidden(
                    "downloads write to the local disk: only the app's own UI can start one",
                ));
            }
            let Command::PodDownloadFile {
                target,
                container,
                path,
                directory,
                destination,
                as_archive,
                size,
            } = command
            else {
                return Err(OxiError::validation("not a download command"));
            };
            let outcome = service
                .download(&DownloadRequest {
                    pod: target,
                    container,
                    path: path.clone(),
                    directory,
                    destination: PathBuf::from(destination),
                    as_archive,
                    total_hint: size,
                })
                .await?;
            Ok(CommandOutput::message(format!(
                "Downloaded {path} ({} bytes in {} file{})",
                outcome.bytes,
                outcome.files,
                if outcome.files == 1 { "" } else { "s" }
            )))
        }
    })?;
    let upload = *command::lookup(CommandId::POD_UPLOAD_FILE)
        .ok_or(RegisterError::Undeclared(CommandId::POD_UPLOAD_FILE))?;
    registry.register(upload, move |command: Command, cx: HandlerContext| {
        let service = service.clone();
        async move {
            let Command::PodUploadFile {
                target,
                container,
                dest_dir,
                local_path,
                size,
                overwrite,
            } = command
            else {
                return Err(OxiError::validation("not an upload command"));
            };
            let mutation = cx.require_mutation()?;
            let receipt = service
                .upload(
                    mutation,
                    &UploadRequest {
                        pod: target,
                        container,
                        dest_dir: dest_dir.clone(),
                        local: PathBuf::from(&local_path),
                        size,
                        overwrite,
                    },
                )
                .await?;
            if receipt.dry_run {
                return Ok(CommandOutput::message(format!(
                    "Dry run: {} would be uploaded to {dest_dir}",
                    file_name(&local_path)
                )));
            }
            cx.note_audit(format!("sent={}", receipt.bytes));
            Ok(CommandOutput::message(format!(
                "Uploaded {} to {dest_dir}",
                file_name(&local_path)
            )))
        }
    })
}

/// The last component of a local path, for messages: the directories of the user's disk are not
/// repeated in the audit log or the confirmation.
fn file_name(local_path: &str) -> String {
    Path::new(local_path)
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// The audit detail of `pod::UploadFile`: the container, the destination, the size and whether it
/// replaces a file. Never the content, and the local directory is left out.
pub fn audit_detail(command: &Command) -> Option<String> {
    let Command::PodUploadFile {
        container,
        dest_dir,
        local_path,
        size,
        overwrite,
        ..
    } = command
    else {
        return None;
    };
    Some(format!(
        "container={} path={} size={size} overwrite={overwrite}",
        container.as_deref().unwrap_or("(default)"),
        super::join(dest_dir, &file_name(local_path)),
    ))
}

/// The confirmation text of `pod::UploadFile`: the local file in full, its size, where it goes.
pub fn summary(command: &Command, context: &str) -> Option<String> {
    let Command::PodUploadFile {
        target,
        container,
        dest_dir,
        local_path,
        size,
        overwrite,
    } = command
    else {
        return None;
    };
    let place = match container {
        Some(container) => format!("container {container} of pod"),
        None => "the default container of pod".to_owned(),
    };
    let object = match target.namespace() {
        Some(ns) => format!("{ns}/{}", target.name),
        None => target.name.to_string(),
    };
    // The whole local path is shown here (a dialog, never a log): the user confirms which file of
    // their disk is read.
    let replaces = if *overwrite {
        " It replaces the file of that name."
    } else {
        ""
    };
    Some(format!(
        "Upload {local_path} ({size} bytes) to {dest_dir} in {place} {object} on {context}.{replaces}"
    ))
}
