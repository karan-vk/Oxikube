//! What the session's Helm adapter can do, so views and tools hide what is missing.

use std::fmt;

/// A `helm` CLI version (`major.minor.patch`; pre-release and build suffixes ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HelmVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch version.
    pub patch: u32,
}

impl HelmVersion {
    /// The oldest `helm` the adapter drives: 3.10 (`--kube-context`, stable `-o json`).
    pub const MINIMUM: HelmVersion = HelmVersion::new(3, 10, 0);

    /// A version from its parts.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses `v3.14.2`, `3.14.2+g1234`, `3.15.0-rc.1` or `3.14`. Missing parts are 0.
    pub fn parse(s: &str) -> Option<Self> {
        let core = s.trim().trim_start_matches('v');
        let core = core.split(['+', '-']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        parts
            .next()
            .is_none()
            .then_some(Self::new(major, minor, patch))
    }

    /// `true` when this version is at least [`MINIMUM`](Self::MINIMUM).
    pub fn is_supported(self) -> bool {
        self >= Self::MINIMUM
    }
}

impl fmt::Display for HelmVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// What the Helm adapter can do right now.
///
/// Native reads (releases, history, values, manifest, notes) need no binary. Everything
/// else needs a supported `helm` CLI. The Helm views and tools key off this: with no
/// binary they hide mutating actions and the chart catalog and show install guidance
/// (E16-S11). It ties into the `helm` [`Capability`](crate::Capability).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HelmCapabilities {
    /// Releases can be read from the cluster's release Secrets/ConfigMaps.
    pub native_read: bool,
    /// A usable `helm` binary (found, and at least [`HelmVersion::MINIMUM`]) is available.
    pub cli_available: bool,
    /// The binary's version, when one was found (even a too-old one).
    pub cli_version: Option<HelmVersion>,
    /// `install` and `template` work.
    pub can_install: bool,
    /// `upgrade` works.
    pub can_upgrade: bool,
    /// `rollback` works.
    pub can_rollback: bool,
    /// `uninstall` works.
    pub can_uninstall: bool,
    /// `repos`, `search` and `show` work.
    pub can_search_repos: bool,
}

impl HelmCapabilities {
    /// Nothing at all: no native reader and no binary.
    pub fn none() -> Self {
        Self::default()
    }

    /// Native reads only: no `helm` binary (or too old a one).
    pub fn native_only() -> Self {
        Self {
            native_read: true,
            ..Self::default()
        }
    }

    /// Native reads plus every CLI operation, with `version` of the binary.
    pub fn full(version: HelmVersion) -> Self {
        Self {
            native_read: true,
            cli_available: true,
            cli_version: Some(version),
            can_install: true,
            can_upgrade: true,
            can_rollback: true,
            can_uninstall: true,
            can_search_repos: true,
        }
    }

    /// `true` when any mutating operation is available.
    pub fn can_mutate(&self) -> bool {
        self.can_install || self.can_upgrade || self.can_rollback || self.can_uninstall
    }
}
