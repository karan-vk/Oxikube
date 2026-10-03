//! Generic Kubernetes object access: [`ResourceReader`], [`ResourceWriter`] and
//! their combination [`ResourcePort`].
//!
//! One instance serves one connected cluster (`ClusterSession`). Every method
//! names its target by [`Gvk`] plus namespace and name; the adapter resolves the
//! REST path through its discovery cache (E03-S06), so CRDs and unknown kinds
//! need no extra code (ADR 0005). Objects come back as domain [`Resource`]s;
//! request bodies go in as raw [`serde_json::Value`] because a new object may
//! not be a valid `Resource` yet (for example `metadata.generateName` without a
//! name).
//!
//! # Mutations
//!
//! Every [`ResourceWriter`] method is mutating: **call it only through
//! `oxikube_app::mutation::MutationGuard`** (non-negotiable 3, ADR 0012). The
//! split exists so the guard can hold the writer privately and hand every other
//! caller only a [`ResourceReader`] (`Arc<dyn ResourcePort>` upcasts to
//! `Arc<dyn ResourceReader>`). Each mutating method takes a dry-run flag
//! ([`WriteOptions::dry_run`], [`DeleteOptions::dry_run`]) so the guard can
//! preview the server's result before executing. Read-only mode is enforced by
//! the guard, not here.
//!
//! # Coverage of kube-rs 4.2 `Api`
//!
//! Reviewed against `kube-client` 4.2.0 `api/core_methods.rs` and
//! `api/subresource.rs` and `kube-core` 4.2.0 `params.rs`.
//!
//! | kube `Api` method | Port method | Notes |
//! |---|---|---|
//! | `list(&ListParams)` | [`ResourceReader::list`] | [`ListOptions`] mirrors `ListParams`; [`ListPage`] carries the continue token |
//! | `list_metadata` | [`ResourceReader::list_metadata`] | metadata-only lists for large clusters |
//! | `get` | [`ResourceReader::get`] | `NotFound` error when absent |
//! | `get_opt` | [`ResourceReader::get_opt`] | `None` when absent |
//! | `get_with(&GetParams)` | — | resource-version reads of one object are not needed by any story; add when one is |
//! | `get_metadata*` | — | a full `get` is one small object; metadata variants only pay off for lists |
//! | `watch`, `watch_metadata` | [`ResourceReader::watch`] | returns a batched [`WatchFeed`]; [`WatchOptions::metadata_only`] selects the metadata variant; raw `WatchEvent`s are adapter-internal |
//! | `create(&PostParams)` | [`ResourceWriter::create`] | `PostParams{dry_run, field_manager}` = [`WriteOptions`] |
//! | `replace(&PostParams)` | [`ResourceWriter::replace`] | optimistic concurrency through `metadata.resourceVersion` in the body |
//! | `patch(&PatchParams, &Patch)` | [`ResourceWriter::patch`] | [`Patch`] carries [`PatchKind`] `Merge`/`Strategic`/`Json`/`Apply{manager, force}` |
//! | `patch_metadata` | [`ResourceWriter::patch`] | a merge patch of `metadata` does the same through the full API |
//! | `delete(&DeleteParams)` | [`ResourceWriter::delete`] | `Either<K, Status>` = [`DeleteOutcome`] |
//! | `delete_collection` | [`ResourceWriter::delete_collection`] | `Either<ObjectList<K>, Status>` = [`DeleteCollectionOutcome`] |
//! | `get_scale` | [`ResourceReader::get_scale`] | typed [`Scale`] |
//! | `patch_scale`, `replace_scale` | [`ResourceWriter::scale`] | sets `spec.replicas`; other scale edits via [`ResourceWriter::patch_subresource`] |
//! | `get_status` | [`ResourceReader::get_subresource`] with [`Subresource::Status`] | |
//! | `patch_status`, `replace_status` | [`ResourceWriter::patch_subresource`], [`ResourceWriter::replace_subresource`] | [`Subresource::Status`] |
//! | `evict(&EvictParams)` | [`ResourceWriter::evict`] | `EvictParams{delete_options, post_options}` = [`DeleteOptions`] (its `dry_run` is the post option) |
//! | `get/patch/replace_ephemeral_containers` | generic subresource methods | [`Subresource::EphemeralContainers`] |
//! | `get/patch/replace_resize` | generic subresource methods | [`Subresource::Resize`] |
//! | `get/create/patch/replace_subresource` | [`ResourceReader::get_subresource`], [`ResourceWriter::create_subresource`], [`ResourceWriter::patch_subresource`], [`ResourceWriter::replace_subresource`] | |
//! | `logs`, `log_stream` | [`LogPort`](crate::log::LogPort) | |
//! | `exec`, `attach` | [`ExecPort`](crate::exec::ExecPort) | |
//! | `portforward` | [`PortForwardPort`](crate::portforward::PortForwardPort) | |
//! | `entry` | — | client-side read-modify-write helper; the app composes `get_opt` + `replace` |
//!
//! Parameters left out on purpose: `PatchParams::field_validation` (the
//! server default, `Warn`, serves every current story; add it when an editor
//! needs `Strict`) and
//! `ListParams::match_any` (expressible as `resource_version: "0"` with
//! [`VersionMatch::NotOlderThan`]).
//!
//! `Patch::Json` needs kube's `jsonpatch` feature; the workspace `kube`
//! dependency already enables it.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OxiResult, Resource};
use serde_json::Value;

use crate::feed::WatchFeed;

/// How the server interprets [`ListOptions::resource_version`]
/// (`resourceVersionMatch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionMatch {
    /// Data at least as new as the given resource version.
    NotOlderThan,
    /// Data at exactly the given resource version (`410 Gone` if compacted).
    Exact,
}

/// Options for [`ResourceReader::list`] and the selector half of
/// [`ResourceWriter::delete_collection`]. Mirrors kube `ListParams`.
///
/// ```ignore
/// let first = ListOptions::default().labels("app=web").limit(500);
/// let next = first.clone().continue_from(page.continue_token.as_deref().unwrap());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListOptions {
    /// Label selector, for example `app=web,tier!=db`.
    pub label_selector: Option<String>,
    /// Field selector, for example `status.phase=Running`.
    pub field_selector: Option<String>,
    /// Page size. `None` asks for everything in one response; prefer a limit
    /// on large collections.
    pub limit: Option<u32>,
    /// Continue token from the previous [`ListPage`].
    pub continue_token: Option<String>,
    /// Resource version to read at (see [`VersionMatch`]).
    pub resource_version: Option<String>,
    /// How `resource_version` is matched.
    pub version_match: Option<VersionMatch>,
    /// Server-side timeout for the call, in seconds.
    pub timeout_secs: Option<u32>,
}

impl ListOptions {
    /// Sets the label selector.
    #[must_use]
    pub fn labels(mut self, selector: impl Into<String>) -> Self {
        self.label_selector = Some(selector.into());
        self
    }

    /// Sets the field selector.
    #[must_use]
    pub fn fields(mut self, selector: impl Into<String>) -> Self {
        self.field_selector = Some(selector.into());
        self
    }

    /// Sets the page size.
    #[must_use]
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Continues a paginated list from `token`.
    #[must_use]
    pub fn continue_from(mut self, token: impl Into<String>) -> Self {
        self.continue_token = Some(token.into());
        self
    }

    /// Reads at `resource_version` with the given match semantics.
    #[must_use]
    pub fn at(mut self, resource_version: impl Into<String>, version_match: VersionMatch) -> Self {
        self.resource_version = Some(resource_version.into());
        self.version_match = Some(version_match);
        self
    }

    /// Sets the server-side timeout.
    #[must_use]
    pub fn timeout_secs(mut self, secs: u32) -> Self {
        self.timeout_secs = Some(secs);
        self
    }
}

/// One page of a list call.
#[derive(Debug, Clone, PartialEq)]
pub struct ListPage<T = Resource> {
    /// The objects in this page.
    pub items: Vec<T>,
    /// Token for the next page; `None` on the last page.
    pub continue_token: Option<String>,
    /// The collection's resource version, usable to start a watch.
    pub resource_version: Option<String>,
    /// Server estimate of the items not yet returned, when it sends one.
    pub remaining_item_count: Option<i64>,
}

impl<T> ListPage<T> {
    /// A single, final page holding `items`.
    pub fn complete(items: Vec<T>) -> Self {
        Self {
            items,
            continue_token: None,
            resource_version: None,
            remaining_item_count: None,
        }
    }

    /// Whether another page follows.
    pub fn has_more(&self) -> bool {
        self.continue_token
            .as_deref()
            .is_some_and(|t| !t.is_empty())
    }
}

/// Options for [`ResourceReader::watch`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchOptions {
    /// Label selector.
    pub label_selector: Option<String>,
    /// Field selector.
    pub field_selector: Option<String>,
    /// Watch `PartialObjectMetadata` only. Each [`Resource`] then holds
    /// metadata, `apiVersion` and `kind` but no spec or status.
    pub metadata_only: bool,
    /// Page size of the initial list. `None` lets the adapter choose.
    pub page_size: Option<u32>,
}

impl WatchOptions {
    /// Sets the label selector.
    #[must_use]
    pub fn labels(mut self, selector: impl Into<String>) -> Self {
        self.label_selector = Some(selector.into());
        self
    }

    /// Sets the field selector.
    #[must_use]
    pub fn fields(mut self, selector: impl Into<String>) -> Self {
        self.field_selector = Some(selector.into());
        self
    }

    /// Watches metadata only.
    #[must_use]
    pub fn metadata_only(mut self) -> Self {
        self.metadata_only = true;
        self
    }

    /// Sets the initial-list page size.
    #[must_use]
    pub fn page_size(mut self, size: u32) -> Self {
        self.page_size = Some(size);
        self
    }
}

/// Options shared by create, replace, patch and the subresource writes.
/// Mirrors kube `PostParams` / `PatchParams`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteOptions {
    /// Server-side dry run (`dryRun=All`): validate and return the would-be
    /// result without persisting it.
    pub dry_run: bool,
    /// Field manager name recorded in `managedFields`. For
    /// [`PatchKind::Apply`] the patch's own manager wins.
    pub field_manager: Option<String>,
}

impl WriteOptions {
    /// Options for a server-side dry run.
    pub fn dry_run() -> Self {
        Self {
            dry_run: true,
            field_manager: None,
        }
    }

    /// Sets the field manager.
    #[must_use]
    pub fn manager(mut self, manager: impl Into<String>) -> Self {
        self.field_manager = Some(manager.into());
        self
    }
}

/// The patch strategy. Mirrors kube `Patch` (whose variants also carry the body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchKind {
    /// JSON merge patch (RFC 7386), `application/merge-patch+json`.
    Merge,
    /// Strategic merge patch, `application/strategic-merge-patch+json`
    /// (built-in kinds only; CRDs reject it).
    Strategic,
    /// JSON patch (RFC 6902), `application/json-patch+json`. The body is the
    /// array of operations.
    Json,
    /// Server-side apply, `application/apply-patch+yaml`.
    Apply {
        /// Field manager that owns the applied fields (required by the server).
        manager: String,
        /// Take ownership of fields held by other managers instead of failing
        /// with a conflict.
        force: bool,
    },
}

impl PatchKind {
    /// The HTTP `Content-Type` for this patch kind.
    pub fn content_type(&self) -> &'static str {
        match self {
            PatchKind::Merge => "application/merge-patch+json",
            PatchKind::Strategic => "application/strategic-merge-patch+json",
            PatchKind::Json => "application/json-patch+json",
            PatchKind::Apply { .. } => "application/apply-patch+yaml",
        }
    }
}

/// A patch: its [`PatchKind`] and its JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    /// The patch strategy.
    pub kind: PatchKind,
    /// The patch document. For [`PatchKind::Json`] an array of operations;
    /// for [`PatchKind::Apply`] the full intended object.
    pub body: Value,
}

impl Patch {
    /// A JSON merge patch.
    pub fn merge(body: Value) -> Self {
        Self {
            kind: PatchKind::Merge,
            body,
        }
    }

    /// A strategic merge patch.
    pub fn strategic(body: Value) -> Self {
        Self {
            kind: PatchKind::Strategic,
            body,
        }
    }

    /// A JSON patch; `ops` is the RFC 6902 operation array.
    pub fn json(ops: Value) -> Self {
        Self {
            kind: PatchKind::Json,
            body: ops,
        }
    }

    /// A server-side apply of `object` owned by `manager`.
    pub fn apply(object: Value, manager: impl Into<String>, force: bool) -> Self {
        Self {
            kind: PatchKind::Apply {
                manager: manager.into(),
                force,
            },
            body: object,
        }
    }
}

/// What happens to an object's dependents on delete. Mirrors kube
/// `PropagationPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropagationPolicy {
    /// Leave dependents in place.
    Orphan,
    /// Delete the object now and dependents in the background.
    Background,
    /// Delete dependents first; the object stays until they are gone.
    Foreground,
}

/// Preconditions the server checks before deleting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preconditions {
    /// Delete only if the object still has this resource version.
    pub resource_version: Option<String>,
    /// Delete only if the object still has this UID (not a re-created namesake).
    pub uid: Option<String>,
}

/// Options for [`ResourceWriter::delete`], [`ResourceWriter::delete_collection`]
/// and [`ResourceWriter::evict`]. Mirrors kube `DeleteParams`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteOptions {
    /// Server-side dry run.
    pub dry_run: bool,
    /// Dependent handling; `None` uses the kind's default.
    pub propagation: Option<PropagationPolicy>,
    /// Grace period in seconds; `Some(0)` deletes immediately.
    pub grace_period_secs: Option<u32>,
    /// Checks the server makes before deleting.
    pub preconditions: Option<Preconditions>,
}

impl DeleteOptions {
    /// Options for a server-side dry run.
    pub fn dry_run() -> Self {
        Self {
            dry_run: true,
            ..Self::default()
        }
    }

    /// Sets the propagation policy.
    #[must_use]
    pub fn propagation(mut self, policy: PropagationPolicy) -> Self {
        self.propagation = Some(policy);
        self
    }

    /// Sets the grace period.
    #[must_use]
    pub fn grace_period_secs(mut self, secs: u32) -> Self {
        self.grace_period_secs = Some(secs);
        self
    }

    /// Sets the preconditions.
    #[must_use]
    pub fn preconditions(mut self, preconditions: Preconditions) -> Self {
        self.preconditions = Some(preconditions);
        self
    }
}

/// Result of [`ResourceWriter::delete`].
#[derive(Debug, Clone, PartialEq)]
pub enum DeleteOutcome {
    /// Deletion started but the object still exists (finalizers, foreground
    /// propagation); this is its current state.
    Deleting(Resource),
    /// The object is gone.
    Deleted,
}

/// Result of [`ResourceWriter::delete_collection`].
#[derive(Debug, Clone, PartialEq)]
pub enum DeleteCollectionOutcome {
    /// The server returned the objects it is deleting (some may still exist).
    Deleting(Vec<Resource>),
    /// The server reported success without listing the objects.
    Deleted,
}

/// A named subresource. `Other` covers anything not listed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subresource {
    /// `status`
    Status,
    /// `scale`
    Scale,
    /// `ephemeralcontainers` (pods)
    EphemeralContainers,
    /// `resize` (pods, in-place resource resize)
    Resize,
    /// Any other subresource, by its path segment.
    Other(Arc<str>),
}

impl Subresource {
    /// The URL path segment, for example `ephemeralcontainers`.
    pub fn as_str(&self) -> &str {
        match self {
            Subresource::Status => "status",
            Subresource::Scale => "scale",
            Subresource::EphemeralContainers => "ephemeralcontainers",
            Subresource::Resize => "resize",
            Subresource::Other(name) => name,
        }
    }
}

impl fmt::Display for Subresource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `scale` subresource (`autoscaling/v1` `Scale`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scale {
    /// Desired replicas (`spec.replicas`).
    pub replicas: i32,
    /// Observed replicas (`status.replicas`).
    pub current_replicas: i32,
    /// Label selector of the scaled pods (`status.selector`), when reported.
    pub selector: Option<String>,
    /// Resource version of the scale object.
    pub resource_version: Option<String>,
}

/// Read access to Kubernetes objects of any kind.
///
/// `namespace` is `None` for cluster-scoped kinds, and for `list`/`watch` of a
/// namespaced kind it means all namespaces. Errors follow the taxonomy in
/// `docs/ARCHITECTURE.md` (`NotFound`, `Forbidden`, `Unsupported` for a kind
/// the server does not serve, ...).
#[async_trait]
pub trait ResourceReader: Send + Sync {
    /// Lists one page of objects. Follow [`ListPage::continue_token`] for the
    /// next page.
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>>;

    /// Lists one page of object metadata (`PartialObjectMetadata`), much
    /// cheaper than [`list`](Self::list) on large collections.
    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>>;

    /// Reads one object. A missing object is an `ErrorKind::NotFound` error.
    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource>;

    /// Reads one object, returning `None` when it does not exist.
    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>>;

    /// Opens a live feed of the kind in `namespace` (or the whole cluster).
    /// The first item is a [`Delta::Restarted`](crate::feed::Delta::Restarted)
    /// with the current list.
    async fn watch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>>;

    /// Reads the `scale` subresource.
    async fn get_scale(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Scale>;

    /// Reads any subresource as raw JSON (for `status`, `ephemeralcontainers`
    /// and `resize` that is the whole object).
    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<Value>;
}

/// Mutating access to Kubernetes objects of any kind.
///
/// **Every method is mutating: call it only through
/// `oxikube_app::mutation::MutationGuard`.** Each takes a dry-run option so
/// the guard can preview the result first.
#[async_trait]
pub trait ResourceWriter: Send + Sync {
    /// Creates an object from `object` (a full manifest).
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn create(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource>;

    /// Replaces an object. A `metadata.resourceVersion` in `object` makes the
    /// write conditional (`Conflict` if it is stale).
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn replace(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource>;

    /// Patches an object. Server-side apply conflicts are `Conflict` errors
    /// naming the other managers unless `force` is set.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn patch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Resource>;

    /// Deletes an object.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn delete(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome>;

    /// Deletes every object matching `selection` in `namespace` (or the whole
    /// cluster). Pagination fields of `selection` are ignored by the server.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn delete_collection(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome>;

    /// Sets `spec.replicas` through the `scale` subresource.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn scale(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        replicas: i32,
        options: &WriteOptions,
    ) -> OxiResult<Scale>;

    /// Evicts a pod through the `eviction` subresource, honouring
    /// PodDisruptionBudgets. A budget refusal (HTTP 429) is a retryable error.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn evict(&self, namespace: &str, pod: &str, options: &DeleteOptions) -> OxiResult<()>;

    /// POSTs `body` to a subresource and returns the server's response.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn create_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value>;

    /// Patches a subresource (`status`, `ephemeralcontainers`, `resize`, ...).
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn patch_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Value>;

    /// Replaces a subresource with `body`.
    ///
    /// Mutating: call only through `MutationGuard`.
    async fn replace_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value>;
}

/// Full object access: [`ResourceReader`] + [`ResourceWriter`].
///
/// Implemented automatically for every type that implements both. Adapters
/// implement the two halves; the app stores `Arc<dyn ResourcePort>` and hands
/// out `Arc<dyn ResourceReader>` (trait upcasting) to everything except
/// `MutationGuard`.
pub trait ResourcePort: ResourceReader + ResourceWriter {}

impl<T: ResourceReader + ResourceWriter + ?Sized> ResourcePort for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_options_builders_set_every_field() {
        let opts = ListOptions::default()
            .labels("app=web")
            .fields("status.phase=Running")
            .limit(500)
            .continue_from("abc")
            .at("123", VersionMatch::NotOlderThan)
            .timeout_secs(30);
        assert_eq!(
            opts,
            ListOptions {
                label_selector: Some("app=web".into()),
                field_selector: Some("status.phase=Running".into()),
                limit: Some(500),
                continue_token: Some("abc".into()),
                resource_version: Some("123".into()),
                version_match: Some(VersionMatch::NotOlderThan),
                timeout_secs: Some(30),
            }
        );
        let exact = ListOptions::default().at("7", VersionMatch::Exact);
        assert_eq!(exact.version_match, Some(VersionMatch::Exact));
    }

    #[test]
    fn list_page_reports_more_pages() {
        let mut page: ListPage<u8> = ListPage::complete(vec![1, 2]);
        assert!(!page.has_more());
        page.continue_token = Some(String::new());
        assert!(!page.has_more());
        page.continue_token = Some("next".into());
        page.resource_version = Some("9".into());
        page.remaining_item_count = Some(10);
        assert!(page.has_more());
    }

    #[test]
    fn watch_options_builders() {
        let opts = WatchOptions::default()
            .labels("a=b")
            .fields("metadata.name=x")
            .metadata_only()
            .page_size(250);
        assert_eq!(
            opts,
            WatchOptions {
                label_selector: Some("a=b".into()),
                field_selector: Some("metadata.name=x".into()),
                metadata_only: true,
                page_size: Some(250),
            }
        );
    }

    #[test]
    fn write_and_delete_options() {
        assert_eq!(
            WriteOptions::dry_run().manager("oxikube"),
            WriteOptions {
                dry_run: true,
                field_manager: Some("oxikube".into()),
            }
        );
        assert!(!WriteOptions::default().dry_run);

        let del = DeleteOptions::dry_run()
            .propagation(PropagationPolicy::Foreground)
            .grace_period_secs(0)
            .preconditions(Preconditions {
                resource_version: Some("5".into()),
                uid: Some("u-1".into()),
            });
        assert!(del.dry_run);
        assert_eq!(del.propagation, Some(PropagationPolicy::Foreground));
        assert_eq!(del.grace_period_secs, Some(0));
        assert_eq!(
            del.preconditions.and_then(|p| p.uid).as_deref(),
            Some("u-1")
        );
        for policy in [
            PropagationPolicy::Orphan,
            PropagationPolicy::Background,
            PropagationPolicy::Foreground,
        ] {
            assert_eq!(
                DeleteOptions::default().propagation(policy).propagation,
                Some(policy)
            );
        }
    }

    #[test]
    fn every_patch_kind_builds_with_its_content_type() {
        let body = json!({"spec": {"replicas": 3}});
        let cases = [
            (
                Patch::merge(body.clone()),
                PatchKind::Merge,
                "application/merge-patch+json",
            ),
            (
                Patch::strategic(body.clone()),
                PatchKind::Strategic,
                "application/strategic-merge-patch+json",
            ),
            (
                Patch::json(json!([{"op": "replace", "path": "/spec/replicas", "value": 3}])),
                PatchKind::Json,
                "application/json-patch+json",
            ),
            (
                Patch::apply(body, "oxikube", true),
                PatchKind::Apply {
                    manager: "oxikube".into(),
                    force: true,
                },
                "application/apply-patch+yaml",
            ),
        ];
        for (patch, kind, content_type) in cases {
            assert_eq!(patch.kind, kind);
            assert_eq!(patch.kind.content_type(), content_type);
        }
        assert!(Patch::json(json!([])).body.is_array());
    }

    #[test]
    fn subresource_names() {
        let cases = [
            (Subresource::Status, "status"),
            (Subresource::Scale, "scale"),
            (Subresource::EphemeralContainers, "ephemeralcontainers"),
            (Subresource::Resize, "resize"),
            (Subresource::Other("binding".into()), "binding"),
        ];
        for (sub, name) in cases {
            assert_eq!(sub.as_str(), name);
            assert_eq!(sub.to_string(), name);
        }
    }

    #[test]
    fn outcomes_and_scale_construct() {
        assert_eq!(DeleteOutcome::Deleted, DeleteOutcome::Deleted);
        assert_eq!(
            DeleteCollectionOutcome::Deleting(Vec::new()),
            DeleteCollectionOutcome::Deleting(Vec::new())
        );
        let scale = Scale {
            replicas: 3,
            current_replicas: 2,
            selector: Some("app=web".into()),
            resource_version: Some("1".into()),
        };
        assert_eq!(scale.replicas, 3);
        assert_eq!(Scale::default().replicas, 0);
    }
}
