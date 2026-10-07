//! Identity types: [`ClusterId`], [`ContextName`], [`Gvk`], [`Gvr`], [`Scope`]
//! and [`ResourceRef`].
//!
//! These are the keys of the ResourceStore and of every watch delta, so they
//! are cheap to clone (`Arc<str>` fields) and hash without allocating. Every
//! type has a stable [`Display`](std::fmt::Display) form and a serde form that
//! reads well in fixtures and in the SQLite state store.
//!
//! # `ResourceRef` text format
//!
//! ```text
//! <cluster-id>/<gvk>/<namespace>/<name>
//! ```
//!
//! where `<gvk>` is `version/Kind` for the core group and `group/version/Kind`
//! otherwise, and `<namespace>` is `-` for cluster-scoped objects. The
//! namespace slot is always present, so the form parses without guessing:
//! splitting on `/` yields 5 segments for a core-group object and 6 otherwise.
//! `-` can never be a Kubernetes namespace name (DNS labels start and end with
//! an alphanumeric), so it is a safe placeholder.
//!
//! ```text
//! 3f2a9c01b7d4e865/v1/Pod/default/web-0
//! 3f2a9c01b7d4e865/apps/v1/Deployment/prod/api
//! 3f2a9c01b7d4e865/v1/Node/-/worker-1
//! ```

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Why an identity string failed to parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdParseError {
    /// A required component was empty.
    #[error("{what} must not be empty")]
    Empty {
        /// Name of the empty component.
        what: &'static str,
    },
    /// The input did not have the expected shape.
    #[error("invalid {what}: {input:?}")]
    Malformed {
        /// Name of the type being parsed.
        what: &'static str,
        /// The offending input.
        input: String,
    },
}

/// A kubeconfig context name (`kubectl config get-contexts`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContextName(Arc<str>);

impl ContextName {
    /// Wrap a context name.
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self(name.into())
    }

    /// The context name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContextName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ContextName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ContextName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ContextName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Domain-separation prefix for [`ClusterId`] hashing; bump the version if the
/// byte layout ever changes.
const CLUSTER_ID_DOMAIN: &[u8] = b"oxikube/cluster-id/v1\0";
/// Number of hash bytes kept in a [`ClusterId`] (16 hex characters).
const CLUSTER_ID_BYTES: usize = 8;

/// Stable identifier of a cluster entry in the catalog: a hash of the
/// kubeconfig source and the context name, as 16 lowercase hex characters.
///
/// The id is stable across runs and platforms. It is derived from an explicit,
/// length-prefixed byte string (never a `Debug` print), so changing either
/// input changes the id and `("ab", "c")` never collides with `("a", "bc")`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClusterId(Arc<str>);

impl ClusterId {
    /// Derive the id from a kubeconfig source and a context name.
    ///
    /// `kubeconfig_source` is hashed verbatim; callers pass a normalised
    /// identifier (for example the canonical absolute path of the kubeconfig
    /// file, or a fixed label such as `in-cluster`). The domain does no I/O,
    /// so it cannot canonicalise paths itself.
    pub fn new(kubeconfig_source: &str, context: &ContextName) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(CLUSTER_ID_DOMAIN);
        for part in [kubeconfig_source, context.as_str()] {
            hasher.update((part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        let digest = hasher.finalize();
        Self(hex::encode(&digest[..CLUSTER_ID_BYTES]).into())
    }

    /// The id as a string slice (16 lowercase hex characters).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClusterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ClusterId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for ClusterId {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let valid = s.len() == CLUSTER_ID_BYTES * 2
            && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if valid {
            Ok(Self(s.into()))
        } else {
            Err(IdParseError::Malformed {
                what: "ClusterId",
                input: s.to_owned(),
            })
        }
    }
}

impl TryFrom<String> for ClusterId {
    type Error = IdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ClusterId> for String {
    fn from(value: ClusterId) -> Self {
        value.0.to_string()
    }
}

/// Split an `apiVersion` (`v1` or `apps/v1`) into `(group, version)`; the core
/// group is the empty string.
fn split_api_version(api_version: &str) -> (&str, &str) {
    api_version.split_once('/').unwrap_or(("", api_version))
}

/// Join a group and version back into an `apiVersion`.
fn join_api_version(group: &str, version: &str) -> String {
    if group.is_empty() {
        version.to_owned()
    } else {
        format!("{group}/{version}")
    }
}

/// Write `[group/]version/last`, omitting the group for the core group.
fn fmt_gv_last(f: &mut fmt::Formatter<'_>, group: &str, version: &str, last: &str) -> fmt::Result {
    if group.is_empty() {
        write!(f, "{version}/{last}")
    } else {
        write!(f, "{group}/{version}/{last}")
    }
}

/// Parse `[group/]version/last` into its three parts (core group is `""`).
fn parse_gv_last<'a>(
    what: &'static str,
    last_what: &'static str,
    s: &'a str,
) -> Result<(&'a str, &'a str, &'a str), IdParseError> {
    let parts: Vec<&str> = s.split('/').collect();
    let (group, version, last) = match parts[..] {
        [version, last] => ("", version, last),
        [group, version, last] => {
            if group.is_empty() {
                return Err(IdParseError::Empty { what: "group" });
            }
            (group, version, last)
        }
        _ => {
            return Err(IdParseError::Malformed {
                what,
                input: s.to_owned(),
            });
        }
    };
    if version.is_empty() {
        return Err(IdParseError::Empty { what: "version" });
    }
    if last.is_empty() {
        return Err(IdParseError::Empty { what: last_what });
    }
    Ok((group, version, last))
}

/// Group-Version-Kind of a Kubernetes type. The core group is the empty string.
///
/// Field order (group, version, kind) drives the derived `Ord`, so sorting is
/// group first, then version, then kind. `Display` is `apps/v1/Deployment`, or
/// `v1/Pod` for the core group.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Gvk {
    /// API group; empty for the core group.
    #[serde(default)]
    pub group: Arc<str>,
    /// API version, for example `v1` or `v1beta1`.
    pub version: Arc<str>,
    /// Kind, for example `Deployment`.
    pub kind: Arc<str>,
}

impl Gvk {
    /// Build a `Gvk` from its parts. Pass `""` as the group for the core group.
    pub fn new(
        group: impl Into<Arc<str>>,
        version: impl Into<Arc<str>>,
        kind: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            kind: kind.into(),
        }
    }

    /// Build a `Gvk` from the `apiVersion` and `kind` fields of a JSON object.
    ///
    /// `"apps/v1"` gives group `apps` and version `v1`; `"v1"` is the core
    /// group. The input is not validated.
    pub fn from_api_version(api_version: &str, kind: &str) -> Self {
        let (group, version) = split_api_version(api_version);
        Self::new(group, version, kind)
    }

    /// The `apiVersion` string of this type (`apps/v1`, or `v1` for core).
    pub fn api_version(&self) -> String {
        join_api_version(&self.group, &self.version)
    }

    /// Whether this type is in the core (legacy) group.
    pub fn is_core(&self) -> bool {
        self.group.is_empty()
    }

    /// Whether this type is the core `v1` Pod.
    pub fn is_pod(&self) -> bool {
        self.is_core() && &*self.kind == "Pod"
    }

    /// Whether this type is the core `v1` Node.
    pub fn is_node(&self) -> bool {
        self.is_core() && &*self.kind == "Node"
    }
}

impl fmt::Display for Gvk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_gv_last(f, &self.group, &self.version, &self.kind)
    }
}

impl FromStr for Gvk {
    type Err = IdParseError;

    /// Parse the [`Display`](fmt::Display) form (`v1/Pod`, `apps/v1/Deployment`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (group, version, kind) = parse_gv_last("Gvk", "kind", s)?;
        Ok(Self::new(group, version, kind))
    }
}

/// Group-Version-Resource: the REST path identity of a type. The core group is
/// the empty string.
///
/// Field order mirrors [`Gvk`]. `Display` is `apps/v1/deployments`, or
/// `v1/pods` for the core group.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Gvr {
    /// API group; empty for the core group.
    #[serde(default)]
    pub group: Arc<str>,
    /// API version, for example `v1`.
    pub version: Arc<str>,
    /// Plural resource name, for example `deployments`.
    pub resource: Arc<str>,
}

impl Gvr {
    /// Build a `Gvr` from its parts. Pass `""` as the group for the core group.
    pub fn new(
        group: impl Into<Arc<str>>,
        version: impl Into<Arc<str>>,
        resource: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            group: group.into(),
            version: version.into(),
            resource: resource.into(),
        }
    }

    /// Build a `Gvr` from an `apiVersion` and a plural resource name.
    pub fn from_api_version(api_version: &str, resource: &str) -> Self {
        let (group, version) = split_api_version(api_version);
        Self::new(group, version, resource)
    }

    /// The `apiVersion` string of this resource (`apps/v1`, or `v1` for core).
    pub fn api_version(&self) -> String {
        join_api_version(&self.group, &self.version)
    }

    /// Whether this resource is in the core (legacy) group.
    pub fn is_core(&self) -> bool {
        self.group.is_empty()
    }
}

impl fmt::Display for Gvr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_gv_last(f, &self.group, &self.version, &self.resource)
    }
}

impl FromStr for Gvr {
    type Err = IdParseError;

    /// Parse the [`Display`](fmt::Display) form (`v1/pods`, `apps/v1/deployments`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (group, version, resource) = parse_gv_last("Gvr", "resource", s)?;
        Ok(Self::new(group, version, resource))
    }
}

/// Whether a kind lives inside a namespace or at cluster level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Cluster-scoped (for example `Node`, `Namespace`).
    Cluster,
    /// Namespaced (for example `Pod`, `Deployment`).
    Namespaced,
}

impl Scope {
    /// `Namespaced` when `namespaced` is true, otherwise `Cluster`.
    pub fn from_namespaced(namespaced: bool) -> Self {
        if namespaced {
            Self::Namespaced
        } else {
            Self::Cluster
        }
    }

    /// Whether objects of this scope live in a namespace.
    pub fn is_namespaced(self) -> bool {
        matches!(self, Self::Namespaced)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cluster => "cluster",
            Self::Namespaced => "namespaced",
        })
    }
}

/// Placeholder written in the namespace slot of a cluster-scoped [`ResourceRef`].
const NO_NAMESPACE: &str = "-";

/// The address of one object: `{cluster, gvk, namespace, name}`.
///
/// `Display` is `<cluster>/<gvk>/<namespace or ->/<name>`; see the
/// [module docs](self) for the exact grammar. The derived `Ord` sorts by
/// cluster, then [`Gvk`], then namespace (cluster-scoped first), then name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceRef {
    /// Cluster the object lives in.
    pub cluster: ClusterId,
    /// Type of the object.
    pub gvk: Gvk,
    /// Namespace, or `None` for cluster-scoped objects. Must not be `"-"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<Arc<str>>,
    /// Object name. Must not contain `/`.
    pub name: Arc<str>,
}

impl ResourceRef {
    /// Build a reference. Pass `None` as the namespace for cluster-scoped objects.
    pub fn new(
        cluster: ClusterId,
        gvk: Gvk,
        namespace: Option<Arc<str>>,
        name: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            cluster,
            gvk,
            namespace,
            name: name.into(),
        }
    }

    /// Build a reference to a namespaced object.
    pub fn namespaced(
        cluster: ClusterId,
        gvk: Gvk,
        namespace: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
    ) -> Self {
        Self::new(cluster, gvk, Some(namespace.into()), name)
    }

    /// Build a reference to a cluster-scoped object.
    pub fn cluster_scoped(cluster: ClusterId, gvk: Gvk, name: impl Into<Arc<str>>) -> Self {
        Self::new(cluster, gvk, None, name)
    }

    /// The namespace as a string slice, if the object is namespaced.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// [`Scope::Namespaced`] when the reference carries a namespace.
    pub fn scope(&self) -> Scope {
        Scope::from_namespaced(self.namespace.is_some())
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.cluster,
            self.gvk,
            self.namespace.as_deref().unwrap_or(NO_NAMESPACE),
            self.name
        )
    }
}

impl FromStr for ResourceRef {
    type Err = IdParseError;

    /// Parse the [`Display`](fmt::Display) form.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('/').collect();
        let (cluster, group, version, kind, namespace, name) = match parts[..] {
            [cluster, version, kind, namespace, name] => {
                (cluster, "", version, kind, namespace, name)
            }
            [cluster, group, version, kind, namespace, name] => {
                if group.is_empty() {
                    return Err(IdParseError::Empty { what: "group" });
                }
                (cluster, group, version, kind, namespace, name)
            }
            _ => {
                return Err(IdParseError::Malformed {
                    what: "ResourceRef",
                    input: s.to_owned(),
                });
            }
        };
        for (what, value) in [
            ("version", version),
            ("kind", kind),
            ("namespace", namespace),
            ("name", name),
        ] {
            if value.is_empty() {
                return Err(IdParseError::Empty { what });
            }
        }
        Ok(Self {
            cluster: cluster.parse()?,
            gvk: Gvk::new(group, version, kind),
            namespace: (namespace != NO_NAMESPACE).then(|| Arc::from(namespace)),
            name: name.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use proptest::prelude::*;

    use super::*;

    fn cid(source: &str, ctx: &str) -> ClusterId {
        ClusterId::new(source, &ContextName::new(ctx))
    }

    #[test]
    fn gvk_display_and_parse_core_and_named_groups() {
        for (text, group, version, kind) in [
            ("v1/Pod", "", "v1", "Pod"),
            ("apps/v1/Deployment", "apps", "v1", "Deployment"),
            (
                "gateway.networking.k8s.io/v1/HTTPRoute",
                "gateway.networking.k8s.io",
                "v1",
                "HTTPRoute",
            ),
        ] {
            let gvk = Gvk::new(group, version, kind);
            assert_eq!(gvk.to_string(), text);
            assert_eq!(text.parse::<Gvk>().unwrap(), gvk);
            assert_eq!(gvk.is_core(), group.is_empty());
        }
    }

    #[test]
    fn gvr_display_and_parse() {
        for (text, group, version, resource) in [
            ("v1/pods", "", "v1", "pods"),
            ("apps/v1/deployments", "apps", "v1", "deployments"),
        ] {
            let gvr = Gvr::new(group, version, resource);
            assert_eq!(gvr.to_string(), text);
            assert_eq!(text.parse::<Gvr>().unwrap(), gvr);
        }
        assert_eq!(
            Gvr::from_api_version("apps/v1", "deployments").api_version(),
            "apps/v1"
        );
    }

    #[test]
    fn gvk_from_api_version() {
        assert_eq!(
            Gvk::from_api_version("v1", "Pod"),
            Gvk::new("", "v1", "Pod")
        );
        assert_eq!(
            Gvk::from_api_version("apps/v1", "Deployment"),
            Gvk::new("apps", "v1", "Deployment")
        );
        assert_eq!(
            Gvk::new("apps", "v1", "Deployment").api_version(),
            "apps/v1"
        );
        assert_eq!(Gvk::new("", "v1", "Pod").api_version(), "v1");
    }

    #[test]
    fn gvk_parse_rejects_bad_input() {
        for bad in ["", "Pod", "a/b/c/d", "/v1/Pod", "v1/", "apps//Deployment"] {
            assert!(bad.parse::<Gvk>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn gvk_ord_is_group_then_version_then_kind() {
        let mut gvks = [
            Gvk::new("batch", "v1", "Job"),
            Gvk::new("apps", "v1", "StatefulSet"),
            Gvk::new("apps", "v1beta1", "Deployment"),
            Gvk::new("apps", "v1", "Deployment"),
            Gvk::new("", "v1", "Pod"),
        ];
        gvks.sort();
        let sorted: Vec<String> = gvks.iter().map(ToString::to_string).collect();
        assert_eq!(
            sorted,
            [
                "v1/Pod",
                "apps/v1/Deployment",
                "apps/v1/StatefulSet",
                "apps/v1beta1/Deployment",
                "batch/v1/Job",
            ]
        );
    }

    #[test]
    fn hash_is_equal_for_equal_values() {
        let a = Gvk::from_api_version("apps/v1", "Deployment");
        let b = Gvk::new(String::from("apps"), "v1", "Deployment");
        let set: HashSet<Gvk> = [a, b].into_iter().collect();
        assert_eq!(set.len(), 1);

        let r1 = ResourceRef::namespaced(cid("a", "x"), Gvk::new("", "v1", "Pod"), "ns", "p");
        let r2 = ResourceRef::namespaced(cid("a", "x"), Gvk::new("", "v1", "Pod"), "ns", "p");
        let set: HashSet<ResourceRef> = [r1, r2].into_iter().collect();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn cluster_id_golden_value() {
        let id = cid("/home/dev/.kube/config", "kind-oxikube");
        assert_eq!(id.as_str(), "46ad12f2e414b3d8");
        assert_eq!(id.to_string().len(), 16);
    }

    #[test]
    fn cluster_id_changes_with_either_input() {
        let base = cid("/home/dev/.kube/config", "kind-a");
        assert_ne!(base, cid("/home/dev/.kube/config", "kind-b"));
        assert_ne!(base, cid("/home/other/.kube/config", "kind-a"));
        assert_eq!(base, cid("/home/dev/.kube/config", "kind-a"));
        // Length prefixes keep the two inputs from bleeding into each other.
        assert_ne!(cid("ab", "c"), cid("a", "bc"));
    }

    #[test]
    fn cluster_id_parse_validates() {
        let id = cid("a", "b");
        assert_eq!(id.as_str().parse::<ClusterId>().unwrap(), id);
        for bad in [
            "",
            "xyz",
            "7B2C2EC86AE1F3B3",
            "7b2c2ec86ae1f3b",
            "7b2c2ec86ae1f3b3a",
        ] {
            assert!(bad.parse::<ClusterId>().is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<ClusterId>(r#""not-an-id""#).is_err());
    }

    #[test]
    fn scope_is_consistent_with_namespaced() {
        assert_eq!(Scope::from_namespaced(true), Scope::Namespaced);
        assert_eq!(Scope::from_namespaced(false), Scope::Cluster);
        assert!(Scope::Namespaced.is_namespaced());
        assert!(!Scope::Cluster.is_namespaced());
        assert_eq!(Scope::Namespaced.to_string(), "namespaced");
    }

    #[test]
    fn resource_ref_display_and_parse() {
        let c = cid("/k", "ctx");
        let cases = [
            (
                ResourceRef::namespaced(c.clone(), Gvk::new("", "v1", "Pod"), "default", "web-0"),
                format!("{c}/v1/Pod/default/web-0"),
            ),
            (
                ResourceRef::namespaced(
                    c.clone(),
                    Gvk::new("apps", "v1", "Deployment"),
                    "prod",
                    "api",
                ),
                format!("{c}/apps/v1/Deployment/prod/api"),
            ),
            (
                ResourceRef::cluster_scoped(c.clone(), Gvk::new("", "v1", "Node"), "worker-1"),
                format!("{c}/v1/Node/-/worker-1"),
            ),
            (
                ResourceRef::cluster_scoped(
                    c.clone(),
                    Gvk::new("rbac.authorization.k8s.io", "v1", "ClusterRole"),
                    "admin",
                ),
                format!("{c}/rbac.authorization.k8s.io/v1/ClusterRole/-/admin"),
            ),
        ];
        for (r, text) in cases {
            assert_eq!(r.to_string(), text);
            assert_eq!(text.parse::<ResourceRef>().unwrap(), r);
        }
    }

    #[test]
    fn resource_ref_parse_rejects_bad_input() {
        let c = cid("/k", "ctx");
        for bad in [
            String::new(),
            "v1/Pod/default/web".to_owned(),
            format!("{c}/v1/Pod/default"),
            format!("{c}/v1/Pod//web"),
            format!("{c}//v1/Pod/default/web"),
            format!("{c}/a/b/c/Pod/default/web"),
            "not-a-cluster/v1/Pod/default/web".to_owned(),
        ] {
            assert!(bad.parse::<ResourceRef>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn resource_ref_scope_and_ord() {
        let c = cid("/k", "ctx");
        let pod = Gvk::new("", "v1", "Pod");
        let ns = ResourceRef::namespaced(c.clone(), pod.clone(), "a", "p");
        let cluster = ResourceRef::cluster_scoped(c, pod, "p");
        assert_eq!(ns.scope(), Scope::Namespaced);
        assert_eq!(cluster.scope(), Scope::Cluster);
        assert_eq!(ns.namespace(), Some("a"));
        assert!(cluster < ns);
    }

    #[test]
    fn serde_round_trips() {
        fn rt<T: Serialize + for<'de> Deserialize<'de> + PartialEq + fmt::Debug>(v: &T) -> String {
            let json = serde_json::to_string(v).unwrap();
            assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), v);
            json
        }
        let c = cid("/k", "ctx");
        assert_eq!(rt(&ContextName::new("kind-oxikube")), "\"kind-oxikube\"");
        assert_eq!(rt(&c), format!("\"{c}\""));
        assert_eq!(
            rt(&Gvk::new("apps", "v1", "Deployment")),
            r#"{"group":"apps","version":"v1","kind":"Deployment"}"#
        );
        rt(&Gvk::new("", "v1", "Pod"));
        rt(&Gvr::new("", "v1", "pods"));
        rt(&Gvr::new("apps", "v1", "deployments"));
        assert_eq!(rt(&Scope::Namespaced), "\"namespaced\"");
        assert_eq!(rt(&Scope::Cluster), "\"cluster\"");
        rt(&ResourceRef::namespaced(
            c.clone(),
            Gvk::new("", "v1", "Pod"),
            "ns",
            "p",
        ));
        let json = rt(&ResourceRef::cluster_scoped(
            c,
            Gvk::new("", "v1", "Node"),
            "n",
        ));
        assert!(!json.contains("namespace"));
    }

    #[test]
    fn gvk_deserializes_without_group() {
        let gvk: Gvk = serde_json::from_str(r#"{"version":"v1","kind":"Pod"}"#).unwrap();
        assert!(gvk.is_core());
    }

    #[test]
    fn gvk_is_pod_is_the_core_pod_only() {
        assert!(Gvk::new("", "v1", "Pod").is_pod());
        assert!(!Gvk::new("metrics.k8s.io", "v1beta1", "Pod").is_pod());
        assert!(!Gvk::new("", "v1", "Node").is_pod());
    }

    proptest! {
        #[test]
        fn gvk_from_api_version_display_round_trips(
            group in proptest::option::of("[a-z0-9]([-a-z0-9.]{0,30}[a-z0-9])?"),
            version in "v[0-9]{1,3}((alpha|beta)[0-9]{1,2})?",
            kind in "[A-Z][A-Za-z0-9]{0,30}",
        ) {
            let api_version = match &group {
                Some(g) => format!("{g}/{version}"),
                None => version.clone(),
            };
            let gvk = Gvk::from_api_version(&api_version, &kind);
            prop_assert_eq!(&*gvk.group, group.as_deref().unwrap_or(""));
            prop_assert_eq!(&*gvk.version, version.as_str());
            prop_assert_eq!(gvk.api_version(), api_version.as_str());
            let shown = gvk.to_string();
            prop_assert_eq!(&shown, &format!("{api_version}/{kind}"));
            prop_assert_eq!(shown.parse::<Gvk>().unwrap(), gvk);
        }

        #[test]
        fn resource_ref_display_round_trips(
            group in proptest::option::of("[a-z0-9]([-a-z0-9.]{0,30}[a-z0-9])?"),
            version in "v[0-9]{1,3}((alpha|beta)[0-9]{1,2})?",
            kind in "[A-Z][A-Za-z0-9]{0,30}",
            namespace in proptest::option::of("[a-z0-9]([-a-z0-9]{0,30}[a-z0-9])?"),
            name in "[a-z0-9]([-a-z0-9.]{0,30}[a-z0-9])?",
            ctx in ".{0,20}",
        ) {
            let gvk = Gvk::new(group.as_deref().unwrap_or(""), version, kind);
            let r = ResourceRef::new(
                ClusterId::new("src", &ContextName::new(ctx)),
                gvk,
                namespace.map(Arc::from),
                name,
            );
            prop_assert_eq!(r.to_string().parse::<ResourceRef>().unwrap(), r);
        }
    }
}
