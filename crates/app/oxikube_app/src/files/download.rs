//! Downloading a file or a directory of a container to the local disk.

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::ExecTransferExt as _;

use super::board::{Direction, SettleOnDrop, TransferHandle, TransferStart};
use super::service::FileService;
use crate::transfer::safe_unpack;

/// What to download.
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    /// The pod.
    pub pod: ResourceRef,
    /// The container; `None` is the pod's default.
    pub container: Option<String>,
    /// The path in the container.
    pub path: String,
    /// Whether `path` is a directory.
    pub directory: bool,
    /// The local file to write, or for a directory the local directory to unpack into.
    pub destination: PathBuf,
    /// Save a directory as one `.tar` file instead of unpacking it.
    pub as_archive: bool,
    /// The size the listing showed, for the progress bar.
    pub total_hint: Option<u64>,
}

/// What a finished download wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    /// Bytes written to disk.
    pub bytes: u64,
    /// Files written (1 for a file or an archive).
    pub files: usize,
    /// Entries of a directory that were not created (links, devices).
    pub skipped: usize,
}

impl FileService {
    /// Downloads a file or directory to the local disk, shown on the [`board`](Self::board).
    ///
    /// A file is fetched with `cat` and written through [`oxikube_ports::FsPort::write_stream`] (a temp file next
    /// to the target, renamed on success: a failure or cancel leaves nothing under the final
    /// name). A directory is a tar stream, unpacked by [`safe_unpack`] or saved as one `.tar`.
    ///
    /// # Errors
    ///
    /// The transfer's own errors (`Unsupported` naming the missing program, `NotFound`,
    /// `Forbidden`, a cancel as `Conflict`) and the file port's.
    pub async fn download(&self, request: &DownloadRequest) -> OxiResult<DownloadOutcome> {
        let local = request
            .destination
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let handle = self.board.begin(TransferStart {
            direction: Direction::Download,
            place: Self::place(&request.pod, request.container.as_deref()),
            remote: request.path.clone(),
            local,
            total: request.total_hint,
        });
        let settle = SettleOnDrop::new(&self.board, &handle);
        let result = self.run_download(request, &handle).await;
        settle.finish(match &result {
            Ok(outcome) => Ok((outcome.skipped > 0).then(|| {
                format!(
                    "{} entries (links or special files) were not created",
                    outcome.skipped
                )
            })),
            Err(error) => Err(error.message().to_owned()),
        });
        result
    }

    async fn run_download(
        &self,
        request: &DownloadRequest,
        handle: &TransferHandle,
    ) -> OxiResult<DownloadOutcome> {
        let (port, _) = self.ports(&request.pod.cluster)?;
        let remote = Self::remote(&request.pod, request.container.as_deref(), &request.path);
        let options = Self::options(handle, request.total_hint);
        let written = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let as_file = |stream: oxikube_ports::exec::ByteStream| {
            let written = written.clone();
            stream
                .map(move |chunk| {
                    let chunk = chunk?;
                    written.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    Ok(chunk.to_vec())
                })
                .boxed()
        };
        if !request.directory {
            let stream = port.cat_download(&remote, options).await?;
            self.fs
                .write_stream(&request.destination, as_file(stream))
                .await?;
            return Ok(DownloadOutcome {
                bytes: written.load(std::sync::atomic::Ordering::Relaxed),
                files: 1,
                skipped: 0,
            });
        }
        let stream = port.tar_download(&remote, options).await?;
        if request.as_archive {
            self.fs
                .write_stream(&request.destination, as_file(stream))
                .await?;
            return Ok(DownloadOutcome {
                bytes: written.load(std::sync::atomic::Ordering::Relaxed),
                files: 1,
                skipped: 0,
            });
        }
        let report = safe_unpack(&*self.fs, &request.destination, stream).await?;
        Ok(DownloadOutcome {
            bytes: report.bytes,
            files: report.files.len(),
            skipped: report.skipped_count,
        })
    }
}
