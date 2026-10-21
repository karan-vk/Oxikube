//! Helm's release storage, read from object metadata alone: drivers, object names, labels,
//! and the "latest revision per release" grouping.
//!
//! Helm 3 and 4 keep one object per revision, named `sh.helm.release.v1.<release>.v<rev>`
//! and labelled `owner=helm`, `name=<release>`, `status=<status>`, `version=<rev>` (plus
//! `modifiedAt=<unix seconds>` once written by Helm 3.x and later). The `secrets` driver
//! (the default) stores them as Secrets of type `helm.sh/release.v1`, the `configmap` driver
//! as ConfigMaps. Nothing here decodes a payload.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ObjectMeta;
use oxikube_domain::helm::ReleaseStatus;
use oxikube_domain::ids::Gvk;

/// The prefix of every release object name.
pub(crate) const NAME_PREFIX: &str = "sh.helm.release.v1.";

/// The label selector that finds release objects of every release.
pub(crate) const OWNER_SELECTOR: &str = "owner=helm";

/// The `type` of a release Secret.
pub(crate) const SECRET_TYPE: &str = "helm.sh/release.v1";

/// Where Helm keeps a release: the storage driver. Ordered Secret first: when both drivers
/// hold the same revision, the Secret wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Driver {
    /// `HELM_DRIVER=secret` (the default): Secrets, payload base64 twice (the API's and
    /// Helm's own encoding).
    Secret,
    /// `HELM_DRIVER=configmap`: ConfigMaps, payload base64 once.
    ConfigMap,
}

impl Driver {
    /// The Kubernetes kind the driver stores revisions as.
    pub(crate) fn gvk(self) -> Gvk {
        match self {
            Driver::Secret => Gvk::new("", "v1", "Secret"),
            Driver::ConfigMap => Gvk::new("", "v1", "ConfigMap"),
        }
    }
}

/// One stored revision as its object metadata describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredRevision {
    /// The driver (and so the kind) of the object.
    pub driver: Driver,
    /// The release's namespace (the object's).
    pub namespace: Arc<str>,
    /// The object's name, `sh.helm.release.v1.<release>.v<rev>`.
    pub object: Arc<str>,
    /// The release name.
    pub release: String,
    /// The revision number.
    pub revision: u32,
    /// The `status` label (the payload is authoritative once decoded).
    pub status: ReleaseStatus,
    /// The `modifiedAt` label, else the object's creation time.
    pub modified: Option<Timestamp>,
    /// Cache key of the object's payload: `(uid, resourceVersion)`. `None` when the server
    /// sent neither, in which case nothing is cached.
    pub cache_key: Option<CacheKey>,
}

/// Identifies one version of one stored object: a new `resourceVersion` means new bytes.
pub(crate) type CacheKey = (Arc<str>, Arc<str>);

/// Splits `sh.helm.release.v1.<release>.v<rev>` into release name and revision. Release
/// names may contain dots (`web.v2` is a valid release name), so the revision is the part
/// after the *last* `.v`.
pub(crate) fn parse_object_name(name: &str) -> Option<(&str, u32)> {
    let rest = name.strip_prefix(NAME_PREFIX)?;
    let (release, revision) = rest.rsplit_once(".v")?;
    let revision = revision.parse().ok().filter(|&r| r > 0)?;
    (!release.is_empty()).then_some((release, revision))
}

/// `true` for a name Helm accepts as a release name (lowercase DNS-1123 subdomain
/// characters): safe to put in a label selector.
pub(crate) fn is_release_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// Reads one listed object's metadata. `None` for an object that is not a release record
/// (wrong name shape, or no `owner=helm` label), so foreign objects never reach the list.
pub(crate) fn parse_stored(meta: &ObjectMeta, driver: Driver) -> Option<StoredRevision> {
    let label = |key: &str| meta.labels.get(key).map(|v| &**v);
    if label("owner") != Some("helm") {
        return None;
    }
    let (name_release, name_revision) = parse_object_name(&meta.name)?;
    // The labels are what `helm` itself selects on; the name is the fallback.
    let release = label("name").unwrap_or(name_release).to_owned();
    let revision = label("version")
        .and_then(|v| v.parse().ok())
        .unwrap_or(name_revision);
    let modified = label("modifiedAt")
        .and_then(|v| v.parse::<i64>().ok())
        .and_then(|secs| Timestamp::from_second(secs).ok())
        .or(meta.creation);
    let cache_key = meta.uid.clone().zip(meta.resource_version.clone());
    Some(StoredRevision {
        driver,
        namespace: meta.namespace.clone()?,
        object: meta.name.clone(),
        release,
        revision,
        status: label("status").map_or(ReleaseStatus::Unknown, ReleaseStatus::parse),
        modified,
        cache_key,
    })
}

/// The latest revision of every release, grouped by `(namespace, name)`. The highest
/// revision number wins (numerically: `v10` is after `v9`); on a tie between drivers the
/// Secret wins. Sorted by namespace, then release name.
pub(crate) fn latest_per_release(stored: Vec<StoredRevision>) -> Vec<StoredRevision> {
    let mut latest: HashMap<(Arc<str>, String), StoredRevision> = HashMap::new();
    for revision in stored {
        let key = (revision.namespace.clone(), revision.release.clone());
        match latest.get(&key) {
            Some(current) if !supersedes(&revision, current) => {}
            _ => {
                latest.insert(key, revision);
            }
        }
    }
    let mut rows: Vec<_> = latest.into_values().collect();
    rows.sort_by(|a, b| (&a.namespace, &a.release).cmp(&(&b.namespace, &b.release)));
    rows
}

/// Every revision of one release, newest first, one per revision number (Secret preferred).
pub(crate) fn history_order(mut stored: Vec<StoredRevision>) -> Vec<StoredRevision> {
    stored.sort_by(|a, b| b.revision.cmp(&a.revision).then(a.driver.cmp(&b.driver)));
    stored.dedup_by_key(|r| r.revision);
    stored
}

fn supersedes(candidate: &StoredRevision, current: &StoredRevision) -> bool {
    (candidate.revision, std::cmp::Reverse(candidate.driver))
        > (current.revision, std::cmp::Reverse(current.driver))
}

#[cfg(test)]
mod tests {
    use oxikube_domain::resource::StrMap;

    use super::*;

    fn object_name(release: &str, revision: u32) -> String {
        format!("{NAME_PREFIX}{release}.v{revision}")
    }

    fn meta(name: &str, namespace: &str, labels: &[(&str, &str)]) -> ObjectMeta {
        let mut meta = ObjectMeta::named(name);
        meta.namespace = Some(namespace.into());
        meta.uid = Some(format!("uid-{name}").into());
        meta.resource_version = Some("7".into());
        meta.labels = labels
            .iter()
            .map(|(k, v)| (Arc::<str>::from(*k), Arc::<str>::from(*v)))
            .collect::<StrMap>();
        meta
    }

    fn stored(namespace: &str, release: &str, revision: u32, driver: Driver) -> StoredRevision {
        let name = object_name(release, revision);
        let version = revision.to_string();
        parse_stored(
            &meta(
                &name,
                namespace,
                &[("owner", "helm"), ("name", release), ("version", &version)],
            ),
            driver,
        )
        .unwrap()
    }

    #[test]
    fn object_names_split_at_the_last_revision_marker() {
        assert_eq!(
            parse_object_name("sh.helm.release.v1.web.v2"),
            Some(("web", 2))
        );
        assert_eq!(
            parse_object_name("sh.helm.release.v1.my.app-1.v10"),
            Some(("my.app-1", 10))
        );
        assert_eq!(
            parse_object_name("sh.helm.release.v1.web.v2.v3"),
            Some(("web.v2", 3))
        );
        assert_eq!(parse_object_name("sh.helm.release.v1.web.v0"), None);
        assert_eq!(parse_object_name("sh.helm.release.v1.web"), None);
        assert_eq!(parse_object_name("sh.helm.release.v1..v1"), None);
        assert_eq!(parse_object_name("default-token-abc"), None);
    }

    #[test]
    fn labels_win_and_foreign_objects_are_ignored() {
        let m = meta(
            "sh.helm.release.v1.web.v3",
            "demo",
            &[
                ("owner", "helm"),
                ("name", "web"),
                ("status", "pending-upgrade"),
                ("version", "3"),
                ("modifiedAt", "1767229200"),
            ],
        );
        let s = parse_stored(&m, Driver::Secret).unwrap();
        assert_eq!((s.release.as_str(), s.revision), ("web", 3));
        assert_eq!(s.status, ReleaseStatus::PendingUpgrade);
        assert_eq!(
            s.modified,
            Some(Timestamp::from_second(1_767_229_200).unwrap())
        );
        assert_eq!(s.cache_key.as_ref().map(|k| &*k.1), Some("7"));

        let not_helm = meta("sh.helm.release.v1.web.v3", "demo", &[("owner", "me")]);
        assert!(parse_stored(&not_helm, Driver::Secret).is_none());
        let not_release = meta("web-config", "demo", &[("owner", "helm")]);
        assert!(parse_stored(&not_release, Driver::ConfigMap).is_none());
        // No labels beyond owner: the name supplies release and revision.
        let bare = meta("sh.helm.release.v1.a.b.v4", "demo", &[("owner", "helm")]);
        let s = parse_stored(&bare, Driver::ConfigMap).unwrap();
        assert_eq!(
            (s.release.as_str(), s.revision, s.status),
            ("a.b", 4, ReleaseStatus::Unknown)
        );
    }

    #[test]
    fn latest_revision_is_numeric_per_namespace_and_name() {
        let rows = latest_per_release(vec![
            stored("a", "web", 9, Driver::Secret),
            stored("a", "web", 10, Driver::Secret),
            stored("a", "web", 2, Driver::Secret),
            stored("b", "web", 1, Driver::Secret),
            stored("a", "db.main", 3, Driver::ConfigMap),
            stored("a", "db.main", 3, Driver::Secret),
        ]);
        let got: Vec<_> = rows
            .iter()
            .map(|r| (&*r.namespace, r.release.as_str(), r.revision, r.driver))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a", "db.main", 3, Driver::Secret),
                ("a", "web", 10, Driver::Secret),
                ("b", "web", 1, Driver::Secret),
            ]
        );
    }

    #[test]
    fn history_is_newest_first_without_duplicate_revisions() {
        let order: Vec<_> = history_order(vec![
            stored("a", "web", 9, Driver::Secret),
            stored("a", "web", 11, Driver::ConfigMap),
            stored("a", "web", 10, Driver::ConfigMap),
            stored("a", "web", 10, Driver::Secret),
        ])
        .into_iter()
        .map(|r| (r.revision, r.driver))
        .collect();
        assert_eq!(
            order,
            vec![
                (11, Driver::ConfigMap),
                (10, Driver::Secret),
                (9, Driver::Secret)
            ]
        );
    }

    #[test]
    fn release_names_safe_for_a_selector() {
        assert!(is_release_name("my.app-1"));
        assert!(!is_release_name(""));
        assert!(!is_release_name("web,owner!=helm"));
        assert!(!is_release_name("Web"));
    }
}
