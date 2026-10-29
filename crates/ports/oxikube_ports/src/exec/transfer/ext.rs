//! [`ExecTransferExt`]: the transfer helpers on every [`ExecStreamPort`].

use async_trait::async_trait;
use oxikube_domain::OxiResult;

use super::super::ExecStreamPort;
use super::failure::Program;
use super::{ByteStream, RemotePath, TransferOptions, download, upload};

/// `kubectl cp` over any [`ExecStreamPort`], including `dyn ExecStreamPort`: a path in a
/// container as a tar stream, and a tar stream unpacked into a container. See the
/// [module docs](super) for how they run and the rules for using them.
///
/// # Effects
///
/// | Method | Effect |
/// |---|---|
/// | [`tar_download`](Self::tar_download), [`cat_download`](Self::cat_download) | none on the cluster (an exec: gate it on the exec policy) |
/// | [`tar_upload`](Self::tar_upload) | **mutates the pod's filesystem**: only through `MutationGuard` |
///
/// # Errors
///
/// The [`ExecStreamPort`] kinds for the open (`NotFound`, `Forbidden`, `Network`...), plus:
/// [`Validation`](oxikube_domain::ErrorKind::Validation) for an empty path or one with `..`;
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) when the container has no `tar`
/// (or no `cat`), naming the container; [`NotFound`](oxikube_domain::ErrorKind::NotFound) when
/// the path is not there; [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) for a permission
/// or read-only file system refusal; [`Conflict`](oxikube_domain::ErrorKind::Conflict) for any
/// other non-zero exit, and once a transfer is cancelled.
#[async_trait]
pub trait ExecTransferExt: ExecStreamPort {
    /// The tar archive of `source.path` (a file or a directory, recursively), as a stream.
    ///
    /// The archive's entries start at the path's last component (`/var/log` gives `log/...`);
    /// never absolute. The stream ends after the remote `tar` exited 0 (warnings ignored), or
    /// with one error: a failed exit mapped as described on the trait, or a cancel. Dropping the
    /// stream stops the remote `tar`. Progress counts archive bytes.
    async fn tar_download(
        &self,
        source: &RemotePath,
        options: TransferOptions,
    ) -> OxiResult<ByteStream> {
        download::open(self, source, Program::TarCreate, options).await
    }

    /// The bytes of the single file `source.path`, with `cat`: the fallback for a container
    /// without `tar` (a distroless image may still have neither). Same stream rules as
    /// [`tar_download`](Self::tar_download), without the archive around the bytes.
    async fn cat_download(
        &self,
        source: &RemotePath,
        options: TransferOptions,
    ) -> OxiResult<ByteStream> {
        download::open(self, source, Program::Cat, options).await
    }

    /// Unpacks the tar `archive` into the directory `dest.path`, which must exist, and returns
    /// the archive bytes sent once the remote `tar` exited 0. Existing files are overwritten;
    /// modification times are not restored.
    ///
    /// **Mutating**: call only through `MutationGuard` (see the module docs). A failure or a
    /// cancel part-way leaves what was unpacked so far.
    async fn tar_upload(
        &self,
        dest: &RemotePath,
        archive: ByteStream,
        options: TransferOptions,
    ) -> OxiResult<u64> {
        upload::upload(self, dest, archive, options).await
    }
}

impl<P: ExecStreamPort + ?Sized> ExecTransferExt for P {}
