//! [`FileService`]: listing, downloading and uploading in pod containers.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecStreamPort, FsPort, RemotePath, ResourceReader, TransferOptions};
use parking_lot::Mutex;

use super::board::{TransferBoard, TransferHandle};
use super::capture::capture;
use super::entry::Listing;
use super::list::list_directory;
use crate::exec::{PodContainers, container_to_open};
use crate::session::ClusterSessionManager;
use crate::transfer::TransferService;

/// How many pods' last directory is kept before the oldest are forgotten.
const MAX_REMEMBERED: usize = 256;

/// The containers of a pod and the one a browser opens first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedPod {
    /// The containers a browser can open.
    pub containers: PodContainers,
    /// The container to open: the one asked for, else the pod's default.
    pub container: Arc<str>,
}

/// Lists, downloads and uploads files of pod containers. See the [module docs](super).
///
/// Plain async Rust over the ports: tests use the fakes. It applies no policy: browsing and
/// downloading are reads, and an upload needs the [`crate::guard::Mutation`] only the guard hands out. Poll
/// its futures from `oxikube_runtime::spawn_kube`, never the UI thread.
pub struct FileService {
    sessions: ClusterSessionManager,
    pub(super) transfer: TransferService,
    pub(super) fs: Arc<dyn FsPort>,
    pub(super) board: TransferBoard,
    last: Mutex<HashMap<(ResourceRef, String), String>>,
}

impl std::fmt::Debug for FileService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileService").finish_non_exhaustive()
    }
}

impl FileService {
    /// A service over the app's sessions, writing and reading local files through `fs`.
    pub fn new(sessions: ClusterSessionManager, fs: Arc<dyn FsPort>) -> Self {
        Self {
            transfer: TransferService::new(sessions.clone()),
            sessions,
            fs,
            board: TransferBoard::new(),
            last: Mutex::new(HashMap::new()),
        }
    }

    /// The transfers of this run.
    pub fn board(&self) -> &TransferBoard {
        &self.board
    }

    pub(super) fn ports(
        &self,
        cluster: &ClusterId,
    ) -> OxiResult<(Arc<dyn ExecStreamPort>, Arc<dyn ResourceReader>)> {
        let session = self
            .sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        let not_connected =
            || OxiError::conflict("the cluster is not connected").with_retryable(true);
        let exec = session.exec_stream().ok_or_else(not_connected)?;
        let reader = session.resources().ok_or_else(not_connected)?;
        Ok((exec, reader))
    }

    /// The containers of `pod` and the one to open: `requested`, else the pod's default.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown pod or container, `Validation` for a pod with no container, and
    /// the read's own errors.
    pub async fn open(&self, pod: &ResourceRef, requested: Option<&str>) -> OxiResult<OpenedPod> {
        let (_, reader) = self.ports(&pod.cluster)?;
        let resource = reader.get(&pod.gvk, pod.namespace(), &pod.name).await?;
        let containers = PodContainers::of(&resource);
        let container = container_to_open(&containers, requested)?;
        Ok(OpenedPod {
            containers,
            container,
        })
    }

    /// The directory listed last in `container` of `pod` in this run.
    pub fn last_path(&self, pod: &ResourceRef, container: &str) -> Option<String> {
        self.last
            .lock()
            .get(&(pod.clone(), container.to_owned()))
            .cloned()
    }

    /// Remembers `path` as the directory to reopen in `container` of `pod` (memory only).
    pub fn remember_path(&self, pod: &ResourceRef, container: &str, path: &str) {
        let mut last = self.last.lock();
        let key = (pod.clone(), container.to_owned());
        if last.len() >= MAX_REMEMBERED && !last.contains_key(&key) {
            last.clear();
        }
        last.insert(key, path.to_owned());
    }

    /// Lists the directory `path` of `container` in `pod`: `ls -la`, else `find`.
    ///
    /// # Errors
    ///
    /// `Unsupported` ([`NO_SHELL_TOOLS`](super::NO_SHELL_TOOLS)) for a container with no shell
    /// tools, `NotFound` for a missing directory, `Forbidden` for one the container's user may not
    /// read, `Validation` for a path that is not a directory, and `NotFound` / `Conflict` when the
    /// cluster is not open or connected.
    pub async fn list(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        path: &str,
    ) -> OxiResult<Listing> {
        let (port, _) = self.ports(&pod.cluster)?;
        let now = jiff::Timestamp::now().as_second();
        list_directory(
            &*port,
            pod.namespace().unwrap_or("default"),
            &pod.name,
            container,
            path,
            now,
        )
        .await
    }

    /// Whether `path` exists in the container (any kind of entry).
    ///
    /// # Errors
    ///
    /// The open's errors, and `Unsupported` when the container has no shell.
    pub async fn exists(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        path: &str,
    ) -> OxiResult<bool> {
        let (port, _) = self.ports(&pod.cluster)?;
        let command: Vec<String> = ["sh", "-c", r#"test -e "$1""#, "sh", path]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let ran = capture(
            &*port,
            pod.namespace().unwrap_or("default"),
            &pod.name,
            container,
            &command,
        )
        .await
        .map_err(|error| {
            if oxikube_ports::exec::error_is_missing_program(&error) {
                OxiError::unsupported(super::list::NO_SHELL_TOOLS)
            } else {
                error
            }
        })?;
        if ran.status.is_missing_program() {
            return Err(OxiError::unsupported(super::list::NO_SHELL_TOOLS));
        }
        Ok(ran.status.is_success())
    }

    pub(super) fn place(pod: &ResourceRef, container: Option<&str>) -> String {
        let ns = pod.namespace().unwrap_or("default");
        match container {
            Some(container) => format!("{ns}/{} ({container})", pod.name),
            None => format!("{ns}/{}", pod.name),
        }
    }

    pub(super) fn remote(pod: &ResourceRef, container: Option<&str>, path: &str) -> RemotePath {
        let remote = RemotePath::new(pod.namespace().unwrap_or("default"), &*pod.name, path);
        match container {
            Some(container) => remote.in_container(container),
            None => remote,
        }
    }

    pub(super) fn options(handle: &TransferHandle, total_hint: Option<u64>) -> TransferOptions {
        TransferOptions {
            total_hint,
            progress: Some(handle.progress.clone()),
            cancel: handle.cancel.clone(),
        }
    }
}
