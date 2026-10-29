//! [`TransferService`]: pod file download and the guarded upload.

use std::sync::Arc;

use futures::{StreamExt as _, stream};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::exec::ByteStream;
use oxikube_ports::{ExecStreamPort, ExecTransferExt as _, RemotePath, TransferOptions};

use crate::guard::Mutation;
use crate::session::ClusterSessionManager;

/// What the caller knows the remote path is (from a listing), which decides the fallback when the
/// container has no `tar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteEntry {
    /// A regular file: downloadable with `cat` when there is no `tar`.
    File,
    /// A directory (or anything else): needs `tar`.
    Directory,
}

/// What a [`Download`] stream holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadFormat {
    /// A tar archive whose entries start at the path's last component. Unpack it with
    /// [`safe_unpack`](super::safe_unpack), never a plain unpack.
    Tar,
    /// The file's bytes as they are (the `cat` fallback): save them to the chosen file.
    Raw,
}

/// A running download.
pub struct Download {
    /// What [`stream`](Self::stream) holds.
    pub format: DownloadFormat,
    /// The bytes; see [`ExecTransferExt::tar_download`] for how it ends. Read it to its end: the
    /// remote program's failure is the last item, after the archive's end marker
    /// ([`safe_unpack`](super::safe_unpack) does).
    ///
    /// [`ExecTransferExt::tar_download`]: oxikube_ports::ExecTransferExt::tar_download
    pub stream: ByteStream,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// What an upload did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadReceipt {
    /// Archive bytes the remote `tar` read (0 for a dry run).
    pub bytes: u64,
    /// The guarded command was a dry run: nothing was sent.
    pub dry_run: bool,
}

/// Downloads from and uploads to pod containers over exec: the session's stream-level exec port
/// with the [`ExecTransferExt`](oxikube_ports::ExecTransferExt) helpers. See the
/// [module docs](super).
///
/// Plain async Rust over the ports, so tests use the fakes. It applies no policy itself: a
/// download is an exec-class read whose command (E20-S07) the guard gates; an upload needs the
/// [`Mutation`] only `MutationGuard` hands to a handler. Poll its futures and streams from
/// `oxikube_runtime::spawn_kube`, never the UI thread.
pub struct TransferService {
    sessions: ClusterSessionManager,
}

impl std::fmt::Debug for TransferService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferService").finish_non_exhaustive()
    }
}

impl TransferService {
    /// A service over the app's sessions.
    pub fn new(sessions: ClusterSessionManager) -> Self {
        Self { sessions }
    }

    fn port(&self, cluster: &ClusterId) -> OxiResult<Arc<dyn ExecStreamPort>> {
        let session = self
            .sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        session
            .exec_stream()
            .ok_or_else(|| OxiError::conflict("the cluster is not connected").with_retryable(true))
    }

    /// Downloads `source` from `cluster`: a tar archive of it, or for a [`RemoteEntry::File`] in
    /// a container without `tar`, the file's bytes through `cat` ([`DownloadFormat::Raw`]).
    ///
    /// The first chunk is awaited before returning, so a download that fails at once (a missing
    /// path, no `tar`) is an `Err` here, before the caller creates anything locally; a failure
    /// later on is the stream's last item.
    ///
    /// # Errors
    ///
    /// `NotFound` / `Conflict` when the cluster is not open or connected, and those of
    /// [`ExecTransferExt::tar_download`]; a container without `tar` is `Unsupported` for a
    /// directory (and for a file whose container has no `cat` either).
    ///
    /// [`ExecTransferExt::tar_download`]: oxikube_ports::ExecTransferExt::tar_download
    pub async fn download(
        &self,
        cluster: &ClusterId,
        source: &RemotePath,
        entry: RemoteEntry,
        options: TransferOptions,
    ) -> OxiResult<Download> {
        let port = self.port(cluster)?;
        let without_tar =
            |error: &OxiError| entry == RemoteEntry::File && error.kind() == ErrorKind::Unsupported;
        let mut archive = match port.tar_download(source, options.clone()).await {
            Ok(archive) => archive,
            Err(error) if without_tar(&error) => return raw(port.as_ref(), source, options).await,
            Err(error) => return Err(error),
        };
        let first = match archive.next().await {
            Some(Err(error)) if without_tar(&error) => {
                return raw(port.as_ref(), source, options).await;
            }
            Some(Err(error)) => return Err(error),
            first => first,
        };
        Ok(Download {
            format: DownloadFormat::Tar,
            stream: stream::iter(first).chain(archive).boxed(),
        })
    }

    /// Unpacks the tar `archive` into the directory `dest` on the cluster `mutation` is for: the
    /// upload behind a guarded command. Only a handler of a mutating command has a [`Mutation`]
    /// (after the read-only check, the confirmation and the audit precondition), so this is the
    /// only way the app writes into a container (non-negotiable 3). A dry run sends nothing.
    ///
    /// # Errors
    ///
    /// `NotFound` / `Conflict` when the cluster is not open or connected, and those of
    /// [`ExecTransferExt::tar_upload`](oxikube_ports::ExecTransferExt::tar_upload).
    pub async fn upload(
        &self,
        mutation: &Mutation,
        dest: &RemotePath,
        archive: ByteStream,
        options: TransferOptions,
    ) -> OxiResult<UploadReceipt> {
        let port = self.port(mutation.cluster())?;
        if mutation.dry_run() {
            return Ok(UploadReceipt {
                bytes: 0,
                dry_run: true,
            });
        }
        let bytes = port.tar_upload(dest, archive, options).await?;
        Ok(UploadReceipt {
            bytes,
            dry_run: false,
        })
    }
}

async fn raw(
    port: &dyn ExecStreamPort,
    source: &RemotePath,
    options: TransferOptions,
) -> OxiResult<Download> {
    Ok(Download {
        format: DownloadFormat::Raw,
        stream: port.cat_download(source, options).await?,
    })
}
