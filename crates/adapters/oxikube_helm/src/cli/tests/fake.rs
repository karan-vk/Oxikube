//! A fake `helm`: a shell script in a temp dir that answers `version --short` and runs a
//! scripted body for everything else, recording its argv and stdin next to itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::ids::{ClusterId, ContextName};
use tempfile::TempDir;

use crate::cli::{HelmCli, HelmCliConfig, HelmTarget, HelmTimeouts};

/// A fake binary and the directory holding it and its recordings.
pub(super) struct FakeHelm {
    dir: TempDir,
}

impl FakeHelm {
    /// A `helm` reporting `version` (`v3.14.2+gabc`) and running `body` (POSIX sh) for any
    /// other command. The argv lands in `argv` (one per line) and stdin in `stdin`.
    pub(super) fn new(version: &str, body: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = format!(
            "#!/bin/sh\nd=\"$(dirname \"$0\")\"\n\
             if [ \"$1\" = version ]; then echo '{version}'; exit 0; fi\n\
             printf '%s\\n' \"$@\" > \"$d/argv\"\n\
             case \" $* \" in *' --values=- '*|*' --password-stdin '*) cat > \"$d/stdin\" ;; esac\n\
             {body}\n"
        );
        install_script(&dir.path().join("helm"), &script);
        Self { dir }
    }

    pub(super) fn path(&self) -> PathBuf {
        self.dir.path().join("helm")
    }

    pub(super) fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// What the last command was given, or `None` when nothing ran.
    pub(super) fn argv(&self) -> Option<Vec<String>> {
        let text = std::fs::read_to_string(self.dir.path().join("argv")).ok()?;
        Some(text.lines().map(str::to_owned).collect())
    }

    pub(super) fn stdin(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("stdin")).ok()
    }

    /// An adapter over this binary with `read` as the read deadline.
    pub(super) fn cli_with(&self, read: Duration) -> HelmCli {
        let config = HelmCliConfig {
            path: Some(self.path()),
            timeouts: HelmTimeouts {
                read,
                mutation_grace: Duration::ZERO,
                ..HelmTimeouts::default()
            },
        };
        HelmCli::new(
            config,
            Arc::new(|_: &ClusterId| Some(HelmTarget::new(ContextName::new("kind-oxikube"), None))),
        )
    }

    pub(super) fn cli(&self) -> HelmCli {
        self.cli_with(Duration::from_secs(10))
    }
}

/// Puts `script` at `path` as an executable without this process ever holding a write fd to
/// `path`. Writing it here directly races with parallel tests: a child forked by another test
/// thread inherits the open write fd until its exec, and exec'ing the script in that window
/// fails with `ETXTBSY` ("Text file busy") on Linux. So the text goes to a sibling `.src` file
/// (never exec'd) and a short-lived `install` child creates the binary; only that child ever
/// opens it for writing, and it has exited before anything runs the script.
pub(super) fn install_script(path: &Path, script: &str) {
    let src = path.with_extension("src");
    std::fs::write(&src, script).unwrap();
    let status = std::process::Command::new("install")
        .arg("-m")
        .arg("755")
        .arg(&src)
        .arg(path)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "install {} failed: {status}",
        path.display()
    );
}

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-oxikube"))
}

/// `true` once the process `pid` has exited (a zombie awaiting the reaper counts), polling for
/// up to 5 s.
pub(super) async fn gone(pid: &str) -> bool {
    for _ in 0..100 {
        let state = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        if state.is_empty() || state.starts_with('Z') {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// The pid the fake wrote to `pid`, waiting up to 5 s for it.
pub(super) async fn pid(fake: &FakeHelm) -> String {
    for _ in 0..100 {
        if let Ok(pid) = std::fs::read_to_string(fake.dir().join("pid"))
            && !pid.trim().is_empty()
        {
            return pid.trim().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the fake helm never started");
}
