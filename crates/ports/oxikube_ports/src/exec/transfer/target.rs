//! [`RemotePath`], [`TransferOptions`], and the commands a transfer runs in the container.

use oxikube_domain::{OxiError, OxiResult};

use super::{CancellationToken, ProgressFn};

/// A path in a container of a pod.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemotePath {
    /// The pod's namespace.
    pub namespace: String,
    /// The pod.
    pub pod: String,
    /// The container; `None` for the pod's only (or default) container.
    pub container: Option<String>,
    /// The path in the container's filesystem: absolute, or relative to the container's working
    /// directory. It may not contain `..` (the archive must not climb out of what was asked for).
    pub path: String,
}

impl RemotePath {
    /// `path` in the default container of `namespace/pod`.
    pub fn new(
        namespace: impl Into<String>,
        pod: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            pod: pod.into(),
            container: None,
            path: path.into(),
        }
    }

    /// The same path in `container`.
    #[must_use]
    pub fn in_container(mut self, container: impl Into<String>) -> Self {
        self.container = Some(container.into());
        self
    }

    /// `container c of pod ns/name`, for messages.
    pub(super) fn place(&self) -> String {
        match &self.container {
            Some(container) => format!(
                "container {container} of pod {}/{}",
                self.namespace, self.pod
            ),
            None => format!("pod {}/{}", self.namespace, self.pod),
        }
    }
}

/// How a transfer reports and stops. `Default` reports nothing and is never cancelled.
#[derive(Clone, Default)]
pub struct TransferOptions {
    /// The size to report as [`TransferProgress::total`](super::TransferProgress::total): the
    /// file sizes the caller saw in a listing, when it has them.
    pub total_hint: Option<u64>,
    /// Where progress goes (throttled to [`PROGRESS_INTERVAL`](super::PROGRESS_INTERVAL), plus a
    /// final report).
    pub progress: Option<ProgressFn>,
    /// Stops the transfer: the exec session is dropped, which stops the remote process.
    pub cancel: CancellationToken,
}

impl std::fmt::Debug for TransferOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferOptions")
            .field("total_hint", &self.total_hint)
            .field("progress", &self.progress.is_some())
            .field("cancel", &self.cancel)
            .finish()
    }
}

/// The error a cancelled transfer ends with. Check the token to tell a cancel from a failure.
pub(super) fn cancelled() -> OxiError {
    OxiError::conflict("the file transfer was cancelled")
}

/// The path's components, checked: not empty, no NUL, no `..`; `.` and empty components dropped.
fn components(path: &str) -> OxiResult<Vec<&str>> {
    if path.trim().is_empty() {
        return Err(OxiError::validation(
            "a path in the container cannot be empty",
        ));
    }
    if path.contains('\0') {
        return Err(OxiError::validation(
            "a path in the container cannot contain NUL",
        ));
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.contains(&"..") {
        return Err(OxiError::validation(format!(
            "{path}: a path in the container cannot contain `..`"
        )));
    }
    Ok(parts)
}

/// A name that cannot be taken for an option.
fn not_an_option(name: String) -> String {
    if name.starts_with('-') {
        format!("./{name}")
    } else {
        name
    }
}

/// Runs its arguments (`"$@"`) and stops them when stdin reaches its end.
///
/// The API server does not stop an exec'd process when the connection closes: a `tar` blocked
/// writing to an output nobody reads would stay in the pod for good. A process reading stdin does
/// see the end, so the script watches stdin (fd 3, a copy, since a background job's own stdin is
/// `/dev/null`) with the shell's `read` and kills the job when it ends. The transfer keeps stdin
/// open and never writes to it: closing it (the stream ends, is cancelled or dropped, or the app
/// dies) stops the remote program. Arguments arrive as `"$@"`, never spliced into the script.
pub(super) const STOP_ON_EOF: &str = "exec 3<&0
\"$@\" </dev/null &
job=$!
{ while read -r _ <&3; do :; done; kill \"$job\"; } 2>/dev/null &
watch=$!
wait \"$job\"
code=$?
kill \"$watch\" 2>/dev/null
exit \"$code\"";

/// `$0` of the [`STOP_ON_EOF`] script, as `ps` shows it.
const SCRIPT_NAME: &str = "oxikube-transfer";

/// `tar -cf - -C <parent> <name>` under [`STOP_ON_EOF`]: the archive of `path`, whose entries
/// start at its last component (never an absolute name, so GNU tar has no leading `/` to strip).
pub(super) fn tar_create_command(path: &str) -> OxiResult<Vec<String>> {
    let parts = components(path)?;
    let absolute = path.starts_with('/');
    let (parent, name) = match parts.split_last() {
        None => (if absolute { "/" } else { "." }.to_owned(), ".".to_owned()),
        Some((name, [])) => (
            if absolute { "/" } else { "." }.to_owned(),
            (*name).to_owned(),
        ),
        Some((name, parent)) => {
            let joined = parent.join("/");
            let parent = if absolute {
                format!("/{joined}")
            } else {
                joined
            };
            (parent, (*name).to_owned())
        }
    };
    Ok(argv(
        [
            "sh",
            "-c",
            STOP_ON_EOF,
            SCRIPT_NAME,
            "tar",
            "-cf",
            "-",
            "-C",
        ],
        [parent, not_an_option(name)],
    ))
}

/// `tar -xmf - -C <dir>`: unpack stdin into `dir` (`m`: do not restore the archive's mtimes,
/// like `kubectl cp`).
pub(super) fn tar_extract_command(dir: &str) -> OxiResult<Vec<String>> {
    components(dir)?;
    Ok(argv(["tar", "-xmf", "-", "-C"], [dir.to_owned()]))
}

/// `cat <file>`: one file's bytes, for a container without `tar`. Run as it is, without the
/// [`STOP_ON_EOF`] guard (an image without `tar` rarely has a shell): a `cat` cancelled while
/// blocked writing may stay in the pod until the container restarts.
pub(super) fn cat_command(file: &str) -> OxiResult<Vec<String>> {
    if components(file)?.is_empty() || file.ends_with('/') {
        return Err(OxiError::validation(format!("{file} is not a file path")));
    }
    Ok(argv(["cat"], [not_an_option(file.to_owned())]))
}

fn argv<const N: usize, const M: usize>(fixed: [&str; N], rest: [String; M]) -> Vec<String> {
    fixed
        .iter()
        .map(|part| (*part).to_owned())
        .chain(rest)
        .collect()
}
