//! API discovery: which kinds a cluster serves.
//!
//! Implemented by `oxikube_kube::discovery` (E03-S06) over aggregated discovery (two
//! requests) with a fallback to the legacy per-group endpoints, and the raw
//! `Client::apiserver_version`. The adapter reads the raw documents because kube's
//! `ApiResource` / `ApiCapabilities` drop short names and categories. The result is the domain [`ResourceKind`]
//! registry record, so nothing above the adapter sees kube's `ApiResource` or
//! `ApiCapabilities`.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
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

/// Which kinds a [`DiscoveryEvent::KindsChanged`] touched. All three lists empty means "something
/// changed and the details were lost" (the subscriber fell behind): re-run discovery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindsChange {
    /// Kinds newly served (a CRD was created).
    pub added: Vec<Gvk>,
    /// Kinds no longer served (a CRD was deleted).
    pub removed: Vec<Gvk>,
    /// Kinds whose record changed (short names, scope, verbs, ...).
    pub changed: Vec<Gvk>,
}

/// Whether the adapter can follow `CustomResourceDefinition` changes. Absence must be visible:
/// a user who may not watch CRDs would otherwise see a registry that silently stops following.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdWatchStatus {
    /// The watch is running (or starting); new CRDs appear without reconnecting.
    Watching,
    /// The API server refused the watch (`Forbidden`). The adapter does not retry in a loop: it
    /// re-discovers on a long interval and tries the watch again then, so the registry still
    /// follows the cluster, slowly.
    Forbidden {
        /// The server's reason, redacted and one line.
        reason: String,
    },
}

impl CrdWatchStatus {
    /// Whether the watch was refused.
    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Forbidden { .. })
    }
}

/// What a [`DiscoveryPort::subscribe`] stream reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// Discovery re-ran after a CRD change and the served kinds differ.
    KindsChanged(KindsChange),
    /// The CRD watch moved to a new status (sent on changes only).
    CrdWatch(CrdWatchStatus),
}

/// The stream of [`DiscoveryEvent`]s. Dropping it stops the watch that feeds it.
pub type DiscoveryEvents = Pin<Box<dyn Stream<Item = DiscoveryEvent> + Send>>;

/// Discovers the kinds a cluster serves. Read-only.
///
/// # Effects
///
/// Read-only.
///
/// # Errors
///
/// Adapters map native failures with the table in `docs/ARCHITECTURE.md`. Expected kinds:
/// [`Auth`](oxikube_domain::ErrorKind::Auth) /
/// [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) for rejected credentials,
/// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) for an API server without discovery
/// endpoints, [`Network`](oxikube_domain::ErrorKind::Network) /
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable) for connection failures. A kind
/// the server does not serve is `Ok(None)` from [`resolve`](Self::resolve), not an error.
#[async_trait]
pub trait DiscoveryPort: Send + Sync {
    /// Runs discovery and returns every served kind, one record per served
    /// group version, with [`ResourceKind::preferred`] set on each kind's
    /// preferred served version (the group's preferred version unless the kind
    /// is only served in another). Subresources (`pods/log`, ...) are not listed.
    async fn discover(&self) -> OxiResult<Vec<ResourceKind>>;

    /// Looks up one kind, from the adapter's cache when it has one. `None`
    /// when the server does not serve `kind`.
    async fn resolve(&self, kind: &Gvk) -> OxiResult<Option<ResourceKind>>;

    /// Reads the API server version.
    async fn server_version(&self) -> OxiResult<ServerVersion>;

    /// Starts following the cluster's kinds: the adapter watches `CustomResourceDefinition`s,
    /// re-runs discovery (so [`resolve`](Self::resolve) answers for a new CRD) and reports each
    /// change. Each call starts its own watch; dropping the stream stops it. Must be called
    /// inside the runtime the adapter's tasks run on. Failures are events
    /// ([`DiscoveryEvent::CrdWatch`]), never a panic or a silent retry loop.
    fn subscribe(&self) -> DiscoveryEvents;
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
