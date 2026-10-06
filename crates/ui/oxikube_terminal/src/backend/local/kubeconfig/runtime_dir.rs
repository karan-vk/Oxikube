//! The per-process runtime directory and the temp kubeconfig files in it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use oxikube_domain::{OxiError, OxiResult};

const DIR_PREFIX: &str = "oxikube-term-";

static RUNTIME_DIR: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// A merged kubeconfig on disk (`0600`, in a `0700` directory). Dropping it deletes the file.
#[derive(Debug)]
pub struct TempKubeconfig {
    path: PathBuf,
}

impl TempKubeconfig {
    /// Writes `contents` to a fresh file in the runtime directory.
    pub(super) fn write(contents: &[u8]) -> OxiResult<Self> {
        let dir = runtime_dir()?;
        let name = format!("kubeconfig-{}", NEXT_FILE.fetch_add(1, Ordering::Relaxed));
        let path = dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(&path)
            .map_err(|_| OxiError::internal("could not create the terminal's kubeconfig"))?;
        // From here on `guard` removes the file, including when the write fails.
        let guard = Self { path };
        file.write_all(contents)
            .map_err(|_| OxiError::internal("could not write the terminal's kubeconfig"))?;
        Ok(guard)
    }

    /// Where the file is.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempKubeconfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Removes this process's runtime directory (call when the app quits; files of terminals still
/// open go with it). A crash leaves it behind: the next start sweeps directories of dead
/// processes.
pub fn cleanup_runtime_dir() {
    if let Some(Ok(dir)) = RUNTIME_DIR.get() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn runtime_dir() -> OxiResult<&'static Path> {
    RUNTIME_DIR
        .get_or_init(|| create_runtime_dir().map_err(|e| e.to_string()))
        .as_deref()
        .map_err(|_| OxiError::internal("could not create the terminal runtime directory"))
}

fn base_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

fn create_runtime_dir() -> std::io::Result<PathBuf> {
    let base = base_dir();
    sweep_stale(&base);
    let dir = base.join(format!("{DIR_PREFIX}{}", std::process::id()));
    // A leftover with our pid (pid reuse after a crash) is not ours to trust.
    let _ = std::fs::remove_dir_all(&dir);
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir)?;
    Ok(dir)
}

/// Removes runtime directories of processes that no longer exist (a crash skips the cleanup).
fn sweep_stale(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix(DIR_PREFIX))
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != std::process::id() && !super::super::unix::process_exists(pid) {
            // Fails harmlessly for a directory that belongs to another user.
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
