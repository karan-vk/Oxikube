//! Uploading one local file into a directory of a container, behind the guard's permit.

use std::path::PathBuf;

use bytes::Bytes;
use futures::StreamExt as _;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};

use super::board::{Direction, SettleOnDrop, TransferStart};
use super::entry::join;
use super::service::FileService;
use crate::guard::Mutation;
use crate::transfer::{UploadReceipt, tar_file_stream};

/// What to upload.
#[derive(Debug, Clone)]
pub struct UploadRequest {
    /// The pod.
    pub pod: ResourceRef,
    /// The container; `None` is the pod's default.
    pub container: Option<String>,
    /// The existing directory the file goes into.
    pub dest_dir: String,
    /// The local file.
    pub local: PathBuf,
    /// The size the caller saw.
    pub size: u64,
    /// The file may replace one of the same name.
    pub overwrite: bool,
}

/// What an upload would do, worked out before the user is asked anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlan {
    /// The local file.
    pub local: PathBuf,
    /// Its name, which is the name in the container.
    pub name: String,
    /// Its size in bytes now.
    pub size: u64,
    /// A file of that name exists in the destination: the upload would replace it.
    pub exists: bool,
}

impl FileService {
    /// Looks at the local file and the destination: its size, and whether the container already
    /// has a file of that name there (the caller asks before replacing it).
    ///
    /// # Errors
    ///
    /// `NotFound` / `Validation` for a local path that is not a file; the destination check's
    /// errors (`Unsupported` for a container with no shell).
    pub async fn plan_upload(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        dest_dir: &str,
        local: &std::path::Path,
    ) -> OxiResult<UploadPlan> {
        let name = local
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| OxiError::validation("the local path is not a file"))?;
        let size = self.fs.read_stream(local).await?.size;
        let exists = self.exists(pod, container, &join(dest_dir, &name)).await?;
        Ok(UploadPlan {
            local: local.to_owned(),
            name,
            size,
            exists,
        })
    }

    /// Uploads one local file into the directory `dest_dir` of a container, as a tar archive
    /// built while it is sent (name and permission bits kept). Runs inside the guarded
    /// `pod::UploadFile` command: `mutation` is the guard's permit.
    ///
    /// # Errors
    ///
    /// `Conflict` when the file changed since the caller measured it, or when the destination
    /// exists and `overwrite` is off; `NotFound` for a missing local file or directory; the
    /// transfer's errors (`Unsupported` for a container without `tar`, `Forbidden` for a
    /// read-only file system or permission).
    pub async fn upload(
        &self,
        mutation: &Mutation,
        request: &UploadRequest,
    ) -> OxiResult<UploadReceipt> {
        let name = request
            .local
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| OxiError::validation("the local path is not a file"))?;
        let container = request.container.as_deref();
        if !request.overwrite
            && self
                .exists(&request.pod, container, &join(&request.dest_dir, &name))
                .await?
        {
            return Err(OxiError::conflict(format!(
                "{name} already exists in {}",
                request.dest_dir
            )));
        }
        let read = self.fs.read_stream(&request.local).await?;
        if read.size != request.size {
            return Err(OxiError::conflict(format!(
                "{name} changed since it was chosen ({} bytes, not {})",
                read.size, request.size
            )));
        }
        let content = read.chunks.map(|chunk| chunk.map(Bytes::from)).boxed();
        let archive = tar_file_stream(&name, read.mode, read.size, content)?;
        let handle = self.board.begin(TransferStart {
            direction: Direction::Upload,
            place: Self::place(&request.pod, container),
            remote: join(&request.dest_dir, &name),
            local: name,
            total: Some(request.size),
        });
        let dest = Self::remote(&request.pod, container, &request.dest_dir);
        let settle = SettleOnDrop::new(&self.board, &handle);
        let result = self
            .transfer
            .upload(
                mutation,
                &dest,
                archive,
                Self::options(&handle, Some(request.size)),
            )
            .await;
        settle.finish(match &result {
            Ok(_) => Ok(None),
            Err(error) => Err(error.message().to_owned()),
        });
        result
    }
}
