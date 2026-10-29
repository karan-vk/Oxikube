//! [`FakeTarExec`]: an in-memory container filesystem that speaks the tar transfer protocol.
//!
//! Where [`FakeExecStreamPort`](super::FakeExecStreamPort) replays scripted sessions, this fake
//! answers the exact commands `ExecTransferExt` runs (`tar_download`, `tar_upload`,
//! `cat_download`; E20-S06) from a small filesystem, so one fake serves the transfer tests of
//! E20-S06, S07 and S10 and a test can check what an upload left behind.
//!
//! | Command | Behaviour |
//! |---|---|
//! | `sh -c <script> .. tar -cf - -C <parent> <name>` | stdout is a real tar (the `tar` crate) of `<parent>/<name>`, entries starting at `<name>` |
//! | `tar -xmf - -C <dir>` | stdin is collected; when it closes the archive is unpacked into `<dir>` |
//! | `cat <file>` | stdout is the file's bytes |
//!
//! Failures behave like GNU tar and a runtime: [`without_tar`](FakeTarExec::without_tar) exits
//! 127 (`executable file not found`), a missing path prints `No such file or directory` and exits
//! 2, a [denied](FakeTarExec::deny) path or a [read-only](FakeTarExec::read_only) container prints
//! `Permission denied` / `Read-only file system` and exits 2.
//!
//! # Limits
//!
//! * Regular files and directories only: links, devices and ownership are not modelled.
//! * Permissions are modes stored and restored, never enforced: a path is unreadable only when
//!   it was [denied](FakeTarExec::deny).
//! * Every pod and container shares the one filesystem; the calls are recorded to tell them apart.
//! * An upload unpacks when stdin closes; a session dropped before that unpacks nothing (real
//!   tar keeps what it already read).
//! * Not a shell: any command other than the three above fails the open with `Validation`.

mod archive;
mod fs;
mod session;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecOptions, ExecSession, ExecStreamPort};
use parking_lot::Mutex;

use fs::MemFs;

#[derive(Default)]
pub(super) struct State {
    fs: MemFs,
    without_tar: bool,
    without_cat: bool,
    read_only: bool,
    denied: Vec<String>,
    calls: Vec<TarExecCall>,
}

/// One command run on a [`FakeTarExec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarExecCall {
    /// Pod namespace.
    pub namespace: String,
    /// Pod name.
    pub pod: String,
    /// The container asked for.
    pub container: Option<String>,
    /// The command line.
    pub command: Vec<String>,
}

/// An in-memory container filesystem behind [`ExecStreamPort`]. See the module docs of `fakes::tar_exec`.
#[derive(Clone, Default)]
pub struct FakeTarExec {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for FakeTarExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("FakeTarExec")
            .field("entries", &state.fs.len())
            .field("calls", &state.calls.len())
            .finish_non_exhaustive()
    }
}

impl FakeTarExec {
    /// An empty filesystem with `tar` and `cat` available.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the file `path` (parents are created) with mode 0644.
    #[must_use]
    pub fn with_file(self, path: &str, content: impl Into<Vec<u8>>) -> Self {
        self.with_file_mode(path, content, 0o644)
    }

    /// Adds the file `path` with `mode`.
    #[must_use]
    pub fn with_file_mode(self, path: &str, content: impl Into<Vec<u8>>, mode: u32) -> Self {
        self.state.lock().fs.put_file(path, content.into(), mode);
        self
    }

    /// Adds the (empty) directory `path`.
    #[must_use]
    pub fn with_dir(self, path: &str) -> Self {
        self.state.lock().fs.put_dir(path, 0o755);
        self
    }

    /// The container has no `tar`: both transfer directions exit 127.
    #[must_use]
    pub fn without_tar(self) -> Self {
        self.state.lock().without_tar = true;
        self
    }

    /// The container has no `cat`: `cat_download` exits 127.
    #[must_use]
    pub fn without_cat(self) -> Self {
        self.state.lock().without_cat = true;
        self
    }

    /// Reading or writing `path` and anything below it fails with `Permission denied`.
    #[must_use]
    pub fn deny(self, path: &str) -> Self {
        self.state.lock().denied.push(fs::normalize(path));
        self
    }

    /// Every write fails with `Read-only file system`.
    #[must_use]
    pub fn read_only(self) -> Self {
        self.state.lock().read_only = true;
        self
    }

    /// The content of the file `path`, if it is one.
    pub fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.state.lock().fs.file(path).map(|(data, _)| data)
    }

    /// The mode of the file or directory `path`.
    pub fn mode(&self, path: &str) -> Option<u32> {
        self.state.lock().fs.mode(path)
    }

    /// Every path in the filesystem, sorted.
    pub fn paths(&self) -> Vec<String> {
        self.state.lock().fs.paths()
    }

    /// The commands run so far, in order.
    pub fn calls(&self) -> Vec<TarExecCall> {
        self.state.lock().calls.clone()
    }
}

#[async_trait]
impl ExecStreamPort for FakeTarExec {
    async fn exec_session(
        &self,
        namespace: &str,
        pod: &str,
        command: &[String],
        options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        self.state.lock().calls.push(TarExecCall {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            container: options.container.clone(),
            command: command.to_vec(),
        });
        session::open(&self.state, command, options)
    }

    async fn attach_session(
        &self,
        _namespace: &str,
        _pod: &str,
        _options: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        Err(OxiError::unsupported("FakeTarExec cannot attach"))
    }
}
