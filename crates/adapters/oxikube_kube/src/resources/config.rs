//! Tuning for [`KubeResources`](super::KubeResources).

use crate::table::TableConfig;

/// Page size of [`KubeResources::list_all`](super::KubeResources::list_all) when the caller
/// sets no limit. 500 keeps one page of raw JSON well under a few MB for pods while a 2 000
/// object list still takes only a handful of requests.
pub const DEFAULT_PAGE_SIZE: u32 = 500;

/// Whether `metadata.managedFields` stays in the converted [`Resource`](oxikube_domain::Resource).
///
/// `managedFields` is large (often more than half of a pod's JSON) and rarely shown, so lists
/// drop it by default; a single `get` keeps it so the YAML editor can show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManagedFields {
    /// Leave `metadata.managedFields` in `Resource::json`.
    Keep,
    /// Remove `metadata.managedFields` before conversion (typed and dynamic paths alike).
    #[default]
    Strip,
}

impl ManagedFields {
    pub(crate) fn strips(self) -> bool {
        self == Self::Strip
    }
}

/// Which `kube::Api` flavour serves a kind.
///
/// Both produce the same [`Resource`](oxikube_domain::Resource) for the same server object, up
/// to explicit `null` members (the kind-cluster golden test compares them); they differ in how
/// the response is decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccessPath {
    /// `Api<DynamicObject>` for every kind. Keeps every field the server sends, including ones
    /// newer than the bundled `k8s-openapi` schema, so it is the default. CRDs always use it.
    #[default]
    Dynamic,
    /// `Api<K>` with the bundled `k8s-openapi` type for the core kinds in `typed::select`
    /// (Pod, Node, Namespace, Service, ConfigMap, Secret, Deployment, StatefulSet, DaemonSet,
    /// ReplicaSet, Job, CronJob, ...); every other kind falls back to [`Dynamic`](Self::Dynamic).
    /// A typed round trip drops fields unknown to the bundled schema and explicit `null`s
    /// (`lastProbeTime: null`), so use it only where that does not matter.
    Typed,
}

/// Settings for one [`KubeResources`](super::KubeResources).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcesConfig {
    /// Page size of `list_all` when [`ListOptions::limit`](oxikube_ports::ListOptions::limit)
    /// is unset. Default [`DEFAULT_PAGE_SIZE`].
    pub page_size: u32,
    /// `managedFields` handling for list and list-all results. Default `Strip`.
    pub list_managed_fields: ManagedFields,
    /// `managedFields` handling for `get` / `get_opt`. Default `Keep`.
    pub get_managed_fields: ManagedFields,
    /// Typed or dynamic decoding. Default `Dynamic`.
    pub access_path: AccessPath,
    /// How often `list_all` (and a Table feed's list) restarts from the first page after the
    /// server expires its continue token (HTTP 410) before giving up. Default 3.
    pub max_restarts: u32,
    /// Table API feed settings (E04-S04).
    pub table: TableConfig,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            list_managed_fields: ManagedFields::Strip,
            get_managed_fields: ManagedFields::Keep,
            access_path: AccessPath::Dynamic,
            max_restarts: 3,
            table: TableConfig::default(),
        }
    }
}
