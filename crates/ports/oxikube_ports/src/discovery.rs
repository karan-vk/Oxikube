//! API discovery: which kinds a cluster serves.
//!
//! Implemented by `oxikube_kube::discovery` (E03-S06) over kube's
//! `Discovery::run_aggregated()` with a fallback to `run()`, and the raw
//! `Client::apiserver_version`. The result is the domain [`ResourceKind`]
//! registry record, so nothing above the adapter sees kube's `ApiResource` or
//! `ApiCapabilities`.

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;

/// The API server's version (`/version`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ServerVersion {
    /// Major version, for example `"1"`.
    pub major: String,
    /// Minor version as reported, for example `"34"` or `"34+"` on some
    /// managed distributions.
    pub minor: String,
    /// Full git version, for example `"v1.34.1"`.
    pub git_version: String,
    /// Build platform, for example `"linux/amd64"`.
    pub platform: String,
}

impl ServerVersion {
    /// `(major, minor)` as numbers, ignoring a trailing `+` on the minor.
    /// `None` when either part is not numeric.
    pub fn numeric(&self) -> Option<(u32, u32)> {
        let major = self.major.trim().parse().ok()?;
        let minor = self.minor.trim().trim_end_matches('+').parse().ok()?;
        Some((major, minor))
    }

    /// Whether the server is at least `major.minor`. `false` when the version
    /// is not numeric.
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        self.numeric().is_some_and(|v| v >= (major, minor))
    }
}

/// Discovers the kinds a cluster serves. Read-only.
#[async_trait]
pub trait DiscoveryPort: Send + Sync {
    /// Runs discovery and returns every served kind, one record per served
    /// group version, with [`ResourceKind::preferred`] set on the group's
    /// preferred version. Subresources (`pods/log`, ...) are not listed.
    async fn discover(&self) -> OxiResult<Vec<ResourceKind>>;

    /// Looks up one kind, from the adapter's cache when it has one. `None`
    /// when the server does not serve `kind`.
    async fn resolve(&self, kind: &Gvk) -> OxiResult<Option<ResourceKind>>;

    /// Reads the API server version.
    async fn server_version(&self) -> OxiResult<ServerVersion>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: &str, minor: &str) -> ServerVersion {
        ServerVersion {
            major: major.into(),
            minor: minor.into(),
            git_version: format!("v{major}.{minor}.0"),
            platform: "linux/amd64".into(),
        }
    }

    #[test]
    fn numeric_version_ignores_plus_suffix() {
        assert_eq!(version("1", "34").numeric(), Some((1, 34)));
        assert_eq!(version("1", "30+").numeric(), Some((1, 30)));
        assert_eq!(version("", "30").numeric(), None);
        assert_eq!(ServerVersion::default().numeric(), None);
    }

    #[test]
    fn at_least_compares_major_then_minor() {
        let v = version("1", "32+");
        assert!(v.at_least(1, 32));
        assert!(v.at_least(1, 27));
        assert!(!v.at_least(1, 33));
        assert!(!v.at_least(2, 0));
        assert!(!version("x", "1").at_least(0, 0));
    }
}
