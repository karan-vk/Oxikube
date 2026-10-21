//! Finding the `helm` binary and gating on its version.

use std::path::{Path, PathBuf};

use oxikube_domain::helm::{HelmCapabilities, HelmError, HelmVersion};
use oxikube_domain::{OxiError, OxiResult};

use super::config::HelmCliConfig;
use super::errors::Redactor;
use super::run::{Invocation, run};

/// Directories searched after `PATH`: an app started from the Dock or a desktop launcher gets a
/// minimal `PATH` that misses the package managers' directories.
const EXTRA_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

/// The executable's file name.
const EXE: &str = if cfg!(windows) { "helm.exe" } else { "helm" };

/// The outcome of the last detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Detection {
    /// Not detected yet: the first operation (or `HelmCli::detect`) runs it.
    Undetected,
    /// A supported binary.
    Ready {
        /// Where it is.
        path: PathBuf,
        /// What `helm version` said.
        version: HelmVersion,
    },
    /// No binary at the configured path, on `PATH` or in the usual directories.
    Missing,
    /// A binary older than [`HelmVersion::MINIMUM`].
    TooOld(HelmVersion),
    /// A binary that would not report a version (redacted reason).
    Broken(String),
}

impl Detection {
    /// What the adapter can do with this binary. The CLI adapter does not read release
    /// Secrets itself, so `native_read` is always `false`; a composite adapter ORs it in.
    pub(crate) fn capabilities(&self) -> HelmCapabilities {
        match self {
            Self::Ready { version, .. } => HelmCapabilities {
                native_read: false,
                ..HelmCapabilities::full(*version)
            },
            Self::TooOld(version) => HelmCapabilities {
                cli_version: Some(*version),
                ..HelmCapabilities::none()
            },
            Self::Undetected | Self::Missing | Self::Broken(_) => HelmCapabilities::none(),
        }
    }

    /// The binary to run, or why there is none.
    pub(crate) fn binary(&self) -> OxiResult<(&Path, HelmVersion)> {
        match self {
            Self::Ready { path, version } => Ok((path, *version)),
            Self::Undetected | Self::Missing => Err(HelmError::BinaryMissing.into()),
            Self::TooOld(found) => Err(HelmError::VersionTooOld {
                found: found.to_string(),
                needed: HelmVersion::MINIMUM.to_string(),
            }
            .into()),
            Self::Broken(reason) => Err(OxiError::unsupported(format!(
                "the helm binary did not report its version: {reason}"
            ))),
        }
    }
}

/// Finds the binary `config` points at and asks it for its version.
pub(crate) async fn detect(config: &HelmCliConfig) -> Detection {
    let Some(path) = locate(config.path.as_deref(), std::env::var_os("PATH").as_deref()) else {
        return Detection::Missing;
    };
    let invocation = Invocation::new(
        vec!["version".into(), "--short".into()],
        config.timeouts.probe,
    );
    match run(&path, invocation).await {
        Ok(out) => match parse_version(&out) {
            Some(version) if version.is_supported() => Detection::Ready { path, version },
            Some(version) => Detection::TooOld(version),
            None => Detection::Broken(format!(
                "unrecognised version output {:?}",
                Redactor::none().apply(out.trim())
            )),
        },
        Err(failure) => Detection::Broken(failure.into_error(None).message().to_owned()),
    }
}

/// The binary: the override when set (it must exist), else the first `helm` on `path_var`,
/// else one in [`EXTRA_DIRS`].
pub(crate) fn locate(
    override_path: Option<&Path>,
    path_var: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return path.is_file().then(|| path.to_path_buf());
    }
    let from_path = path_var.into_iter().flat_map(std::env::split_paths);
    from_path
        .chain(EXTRA_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join(EXE))
        .find(|candidate| candidate.is_file())
}

/// Parses `helm version --short` (`v3.10.3+g835b733`; Helm 2 printed `Client: v2.16.1+g...`).
pub(crate) fn parse_version(out: &str) -> Option<HelmVersion> {
    let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.strip_prefix("Client:").map_or(line, str::trim);
    HelmVersion::parse(line)
}
