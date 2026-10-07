//! [`FakeResourcePort`]: `ResourceReader` + `ResourceWriter` over an in-memory object
//! store, with scripted responses and timed watch replay.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{ObjectMeta, OxiError, OxiResult, Resource};
use oxikube_ports::{
    ClockPort, DeleteCollectionOutcome, DeleteOptions, DeleteOutcome, Delta, DeltaBatch,
    ListOptions, ListPage, Patch, ResourceReader, ResourceWriter, Scale, Subresource, WatchFeed,
    WatchOptions, WriteOptions,
};
use parking_lot::Mutex;
use serde_json::Value;

use super::FakeClockPort;
use crate::script::{CallLog, Script, StreamGauge, Timeline};

const PORT: &str = "FakeResourcePort";

/// Queued responses for each [`FakeResourcePort`] method.
#[derive(Debug, Default)]
pub struct ResourceScripts {
    /// `ResourceReader::list`.
    pub list: Script<ListPage<Resource>>,
    /// `ResourceReader::list_metadata`.
    pub list_metadata: Script<ListPage<ObjectMeta>>,
    /// `ResourceReader::get`.
    pub get: Script<Resource>,
    /// `ResourceReader::get_opt`.
    pub get_opt: Script<Option<Resource>>,
    /// `ResourceReader::watch`: each entry is the timeline one watch stream replays
    /// (or the error `watch` itself returns).
    pub watch: Script<Timeline<DeltaBatch<Resource>>>,
    /// `ResourceReader::get_scale`.
    pub get_scale: Script<Scale>,
    /// `ResourceReader::get_subresource`.
    pub get_subresource: Script<Value>,
    /// `ResourceWriter::create`.
    pub create: Script<Resource>,
    /// `ResourceWriter::replace`.
    pub replace: Script<Resource>,
    /// `ResourceWriter::patch`.
    pub patch: Script<Resource>,
    /// `ResourceWriter::delete`.
    pub delete: Script<DeleteOutcome>,
    /// `ResourceWriter::delete_collection`.
    pub delete_collection: Script<DeleteCollectionOutcome>,
    /// `ResourceWriter::scale`.
    pub scale: Script<Scale>,
    /// `ResourceWriter::evict`.
    pub evict: Script<()>,
    /// `ResourceWriter::create_subresource`.
    pub create_subresource: Script<Value>,
    /// `ResourceWriter::patch_subresource`.
    pub patch_subresource: Script<Value>,
    /// `ResourceWriter::replace_subresource`.
    pub replace_subresource: Script<Value>,
}

/// One call made on a [`FakeResourcePort`], with its arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceCall {
    /// `list`.
    List {
        /// Kind listed.
        kind: Gvk,
        /// Namespace, `None` for all namespaces or cluster-scoped kinds.
        namespace: Option<String>,
        /// Options passed.
        options: ListOptions,
    },
    /// `list_metadata`.
    ListMetadata {
        /// Kind listed.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Options passed.
        options: ListOptions,
    },
    /// `get`.
    Get {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
    },
    /// `get_opt`.
    GetOpt {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
    },
    /// `watch`.
    Watch {
        /// Kind watched.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Options passed.
        options: WatchOptions,
    },
    /// `get_scale`.
    GetScale {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
    },
    /// `get_subresource`.
    GetSubresource {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Subresource read.
        subresource: Subresource,
    },
    /// `create` (mutating).
    Create {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Request body.
        object: Value,
        /// Options passed.
        options: WriteOptions,
    },
    /// `replace` (mutating).
    Replace {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Request body.
        object: Value,
        /// Options passed.
        options: WriteOptions,
    },
    /// `patch` (mutating).
    Patch {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Patch sent.
        patch: Patch,
        /// Options passed.
        options: WriteOptions,
    },
    /// `delete` (mutating).
    Delete {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Options passed.
        options: DeleteOptions,
    },
    /// `delete_collection` (mutating).
    DeleteCollection {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Which objects.
        selection: ListOptions,
        /// Options passed.
        options: DeleteOptions,
    },
    /// `scale` (mutating).
    Scale {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Requested replicas.
        replicas: i32,
        /// Options passed.
        options: WriteOptions,
    },
    /// `evict` (mutating).
    Evict {
        /// Pod namespace.
        namespace: String,
        /// Pod name.
        pod: String,
        /// Options passed.
        options: DeleteOptions,
    },
    /// `create_subresource` (mutating).
    CreateSubresource {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Subresource.
        subresource: Subresource,
        /// Request body.
        body: Value,
        /// Options passed.
        options: WriteOptions,
    },
    /// `patch_subresource` (mutating).
    PatchSubresource {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Subresource.
        subresource: Subresource,
        /// Patch sent.
        patch: Patch,
        /// Options passed.
        options: WriteOptions,
    },
    /// `replace_subresource` (mutating).
    ReplaceSubresource {
        /// Kind.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Object name.
        name: String,
        /// Subresource.
        subresource: Subresource,
        /// Request body.
        body: Value,
        /// Options passed.
        options: WriteOptions,
    },
}

impl ResourceCall {
    /// `true` for `ResourceWriter` calls, the ones `MutationGuard` must gate.
    pub fn is_mutating(&self) -> bool {
        !matches!(
            self,
            Self::List { .. }
                | Self::ListMetadata { .. }
                | Self::Get { .. }
                | Self::GetOpt { .. }
                | Self::Watch { .. }
                | Self::GetScale { .. }
                | Self::GetSubresource { .. }
        )
    }

    /// `true` when the call is a mutation sent as a server-side dry run.
    pub fn is_dry_run(&self) -> bool {
        match self {
            Self::Create { options, .. }
            | Self::Replace { options, .. }
            | Self::Patch { options, .. }
            | Self::Scale { options, .. }
            | Self::CreateSubresource { options, .. }
            | Self::PatchSubresource { options, .. }
            | Self::ReplaceSubresource { options, .. } => options.dry_run,
            Self::Delete { options, .. }
            | Self::DeleteCollection { options, .. }
            | Self::Evict { options, .. } => options.dry_run,
            _ => false,
        }
    }
}

/// Fake `ResourcePort` (`ResourceReader` + `ResourceWriter`).
///
/// Holds an in-memory object store seeded with [`with_objects`](Self::with_objects) or
/// [`insert`](Self::insert). Fallbacks when a method's script is empty:
///
/// * `list`, `list_metadata`, `get`, `get_opt` read the store (`get` of a missing object
///   is `NotFound`). Label selectors support `k=v`, `k==v`, `k!=v`, `k` and `!k`; field
///   selectors support `metadata.name` and `metadata.namespace` equality.
/// * `watch` replays one `Restarted` batch with the matching stored objects, then stays
///   open.
/// * `create` / `replace` parse the body into a `Resource`, store it (unless dry run) and
///   return it; `create` of an existing object is `Conflict`, `replace` of a missing one
///   `NotFound`. `delete` removes the object (unless dry run) and returns `Deleted`.
/// * Every other method returns the [`unscripted`](crate::unscripted) error.
///
/// Scripted watch timelines are replayed against [`clock`](Self::clock): offsets count
/// from the `watch` call, and nothing is delivered until the test advances the clock.
pub struct FakeResourcePort {
    script: ResourceScripts,
    calls: CallLog<ResourceCall>,
    store: Mutex<Vec<Resource>>,
    clock: Arc<FakeClockPort>,
    watches: StreamGauge,
}

fake_plumbing!(FakeResourcePort, ResourceScripts, ResourceCall);

impl Default for FakeResourcePort {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for FakeResourcePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeResourcePort")
            .field("objects", &self.store.lock().len())
            .field("calls", &self.calls.len())
            .finish_non_exhaustive()
    }
}

impl FakeResourcePort {
    /// An empty fake with its own [`FakeClockPort`].
    pub fn new() -> Self {
        Self::with_clock(Arc::new(FakeClockPort::default()))
    }

    /// An empty fake whose watch replays are timed on `clock`.
    pub fn with_clock(clock: Arc<FakeClockPort>) -> Self {
        Self {
            script: ResourceScripts::default(),
            calls: CallLog::default(),
            store: Mutex::new(Vec::new()),
            clock,
            watches: StreamGauge::default(),
        }
    }

    /// Seeds the object store.
    #[must_use]
    pub fn with_objects(self, objects: impl IntoIterator<Item = Resource>) -> Self {
        for object in objects {
            self.insert(object);
        }
        self
    }

    /// The clock watch replays are timed on; advance it to deliver scripted events.
    pub fn clock(&self) -> &Arc<FakeClockPort> {
        &self.clock
    }

    /// Watch streams handed out by `watch` that the caller has not dropped yet: opened feeds
    /// minus stopped ones.
    pub fn live_watches(&self) -> usize {
        self.watches.live()
    }

    /// Adds `object` to the store, replacing an object with the same kind, namespace and
    /// name.
    pub fn insert(&self, object: Resource) {
        let mut store = self.store.lock();
        match store.iter_mut().find(|o| same_object(o, &object)) {
            Some(existing) => *existing = object,
            None => store.push(object),
        }
    }

    /// Removes the object of `kind` named `name` in `namespace` from the store (it was deleted
    /// behind the code under test's back). Returns whether there was one. Not a recorded call.
    pub fn remove(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> bool {
        let mut store = self.store.lock();
        let before = store.len();
        store.retain(|o| !(&o.kind == kind && o.namespace() == namespace && o.name() == name));
        store.len() != before
    }

    /// A copy of every stored object, in insertion order.
    pub fn objects(&self) -> Vec<Resource> {
        self.store.lock().clone()
    }

    /// The recorded `ResourceWriter` calls only.
    pub fn mutating_calls(&self) -> Vec<ResourceCall> {
        self.recorded_calls()
            .into_iter()
            .filter(ResourceCall::is_mutating)
            .collect()
    }

    fn find(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> Option<Resource> {
        self.store
            .lock()
            .iter()
            .find(|o| &o.kind == kind && o.namespace() == namespace && o.name() == name)
            .cloned()
    }

    fn select(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        labels: Option<&str>,
        fields: Option<&str>,
    ) -> OxiResult<Vec<Resource>> {
        let labels = labels.map(LabelSelector::parse).transpose()?;
        let fields = fields.map(parse_field_selector).transpose()?;
        Ok(self
            .store
            .lock()
            .iter()
            .filter(|o| &o.kind == kind)
            .filter(|o| namespace.is_none_or(|ns| o.namespace() == Some(ns)))
            .filter(|o| labels.as_ref().is_none_or(|s| s.matches(&o.meta)))
            .filter(|o| {
                fields.as_ref().is_none_or(|f| {
                    f.iter().all(|(field, value)| match field.as_str() {
                        "metadata.name" => o.name() == value,
                        _ => o.namespace() == Some(value.as_str()),
                    })
                })
            })
            .cloned()
            .collect())
    }

    fn list_store(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<Vec<Resource>> {
        self.select(
            kind,
            namespace,
            options.label_selector.as_deref(),
            options.field_selector.as_deref(),
        )
    }
}

fn same_object(a: &Resource, b: &Resource) -> bool {
    a.kind == b.kind && a.namespace() == b.namespace() && a.name() == b.name()
}

fn owned(namespace: Option<&str>) -> Option<String> {
    namespace.map(str::to_owned)
}

fn object_from_body(object: &Value, namespace: Option<&str>) -> OxiResult<Resource> {
    let mut object = object.clone();
    // The server names an object that carries `generateName` and no name (a node shell's pod).
    if let Some(meta) = object.get_mut("metadata").and_then(Value::as_object_mut)
        && !meta.contains_key("name")
        && let Some(prefix) = meta.get("generateName").and_then(Value::as_str)
    {
        let name = format!("{prefix}x7k2p");
        meta.insert("name".into(), Value::String(name));
    }
    let mut res = Resource::from_json(object)?;
    if res.meta.namespace.is_none() {
        if let Some(ns) = namespace {
            res.meta.namespace = Some(Arc::from(ns));
            if let Some(meta) = res
                .json_mut()
                .get_mut("metadata")
                .and_then(Value::as_object_mut)
            {
                meta.insert("namespace".into(), Value::String(ns.to_owned()));
            }
        }
    }
    Ok(res)
}

/// A parsed equality/existence label selector (`a=b,c!=d,e,!f`).
struct LabelSelector(Vec<LabelTerm>);

enum LabelTerm {
    Eq(String, String),
    Ne(String, String),
    Exists(String),
    Absent(String),
}

impl LabelSelector {
    fn parse(selector: &str) -> OxiResult<Self> {
        let mut terms = Vec::new();
        for raw in selector.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let term = if let Some((k, v)) = raw.split_once("!=") {
                LabelTerm::Ne(k.trim().into(), v.trim().into())
            } else if let Some((k, v)) = raw.split_once("==").or_else(|| raw.split_once('=')) {
                LabelTerm::Eq(k.trim().into(), v.trim().into())
            } else if let Some(k) = raw.strip_prefix('!') {
                LabelTerm::Absent(k.trim().into())
            } else if raw.contains(' ') || raw.contains('(') {
                return Err(OxiError::unsupported(format!(
                    "{PORT}: set-based label selector {raw:?} is not supported"
                )));
            } else {
                LabelTerm::Exists(raw.into())
            };
            terms.push(term);
        }
        Ok(Self(terms))
    }

    fn matches(&self, meta: &ObjectMeta) -> bool {
        self.0.iter().all(|term| match term {
            LabelTerm::Eq(k, v) => meta.labels.get(k.as_str()).is_some_and(|x| &**x == v),
            LabelTerm::Ne(k, v) => meta.labels.get(k.as_str()).is_none_or(|x| &**x != v),
            LabelTerm::Exists(k) => meta.labels.contains_key(k.as_str()),
            LabelTerm::Absent(k) => !meta.labels.contains_key(k.as_str()),
        })
    }
}

fn parse_field_selector(selector: &str) -> OxiResult<Vec<(String, String)>> {
    selector
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|term| {
            let (k, v) = term
                .split_once("==")
                .or_else(|| term.split_once('='))
                .ok_or_else(|| OxiError::unsupported(format!("{PORT}: field selector {term:?}")))?;
            let k = k.trim();
            if k == "metadata.name" || k == "metadata.namespace" {
                Ok((k.to_owned(), v.trim().to_owned()))
            } else {
                Err(OxiError::unsupported(format!(
                    "{PORT}: field selector on {k:?} is not supported (script `list` instead)"
                )))
            }
        })
        .collect()
}

#[async_trait]
impl ResourceReader for FakeResourcePort {
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        self.calls.record(ResourceCall::List {
            kind: kind.clone(),
            namespace: owned(namespace),
            options: options.clone(),
        });
        self.script.list.next_or_else(|| {
            self.list_store(kind, namespace, options)
                .map(ListPage::complete)
        })
    }

    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        self.calls.record(ResourceCall::ListMetadata {
            kind: kind.clone(),
            namespace: owned(namespace),
            options: options.clone(),
        });
        self.script.list_metadata.next_or_else(|| {
            let items = self.list_store(kind, namespace, options)?;
            Ok(ListPage::complete(
                items.into_iter().map(|o| o.meta).collect(),
            ))
        })
    }

    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource> {
        self.calls.record(ResourceCall::Get {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
        });
        self.script.get.next_or_else(|| {
            self.find(kind, namespace, name).ok_or_else(|| {
                OxiError::not_found(format!(
                    "{kind} {}/{name} not found",
                    namespace.unwrap_or("")
                ))
            })
        })
    }

    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        self.calls.record(ResourceCall::GetOpt {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
        });
        self.script
            .get_opt
            .next_or_else(|| Ok(self.find(kind, namespace, name)))
    }

    async fn watch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        self.calls.record(ResourceCall::Watch {
            kind: kind.clone(),
            namespace: owned(namespace),
            options: options.clone(),
        });
        let timeline = self.script.watch.next_or_else(|| {
            let objects = self.select(
                kind,
                namespace,
                options.label_selector.as_deref(),
                options.field_selector.as_deref(),
            )?;
            let objects = if options.metadata_only {
                objects.into_iter().map(metadata_only).collect()
            } else {
                objects
            };
            let batch = DeltaBatch::from_deltas(vec![Delta::Restarted(objects)]);
            Ok(Timeline::immediate([batch]).keep_open())
        })?;
        let clock: Arc<dyn ClockPort> = self.clock.clone();
        Ok(self.watches.track(timeline.replay(clock)))
    }

    async fn get_scale(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Scale> {
        self.calls.record(ResourceCall::GetScale {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
        });
        self.script.get_scale.next_or_unscripted(PORT, "get_scale")
    }

    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<Value> {
        self.calls.record(ResourceCall::GetSubresource {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            subresource: subresource.clone(),
        });
        self.script
            .get_subresource
            .next_or_unscripted(PORT, "get_subresource")
    }
}

#[async_trait]
impl ResourceWriter for FakeResourcePort {
    async fn create(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.calls.record(ResourceCall::Create {
            kind: kind.clone(),
            namespace: owned(namespace),
            object: object.clone(),
            options: options.clone(),
        });
        self.script.create.next_or_else(|| {
            let res = object_from_body(object, namespace)?;
            if self.find(&res.kind, res.namespace(), res.name()).is_some() {
                return Err(OxiError::conflict(format!(
                    "{} {} already exists",
                    res.kind,
                    res.name()
                )));
            }
            if !options.dry_run {
                self.insert(res.clone());
            }
            Ok(res)
        })
    }

    async fn replace(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        object: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.calls.record(ResourceCall::Replace {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            object: object.clone(),
            options: options.clone(),
        });
        self.script.replace.next_or_else(|| {
            if self.find(kind, namespace, name).is_none() {
                return Err(OxiError::not_found(format!("{kind} {name} not found")));
            }
            let res = object_from_body(object, namespace)?;
            if !options.dry_run {
                self.insert(res.clone());
            }
            Ok(res)
        })
    }

    async fn patch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Resource> {
        self.calls.record(ResourceCall::Patch {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            patch: patch.clone(),
            options: options.clone(),
        });
        self.script.patch.next_or_unscripted(PORT, "patch")
    }

    async fn delete(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        self.calls.record(ResourceCall::Delete {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            options: options.clone(),
        });
        self.script.delete.next_or_else(|| {
            if self.find(kind, namespace, name).is_none() {
                return Err(OxiError::not_found(format!("{kind} {name} not found")));
            }
            if !options.dry_run {
                self.store.lock().retain(|o| {
                    !(&o.kind == kind && o.namespace() == namespace && o.name() == name)
                });
            }
            Ok(DeleteOutcome::Deleted)
        })
    }

    async fn delete_collection(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        selection: &ListOptions,
        options: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        self.calls.record(ResourceCall::DeleteCollection {
            kind: kind.clone(),
            namespace: owned(namespace),
            selection: selection.clone(),
            options: options.clone(),
        });
        self.script
            .delete_collection
            .next_or_unscripted(PORT, "delete_collection")
    }

    async fn scale(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        replicas: i32,
        options: &WriteOptions,
    ) -> OxiResult<Scale> {
        self.calls.record(ResourceCall::Scale {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            replicas,
            options: options.clone(),
        });
        self.script.scale.next_or_unscripted(PORT, "scale")
    }

    async fn evict(&self, namespace: &str, pod: &str, options: &DeleteOptions) -> OxiResult<()> {
        self.calls.record(ResourceCall::Evict {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            options: options.clone(),
        });
        self.script.evict.next_or_unscripted(PORT, "evict")
    }

    async fn create_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.calls.record(ResourceCall::CreateSubresource {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            subresource: subresource.clone(),
            body: body.clone(),
            options: options.clone(),
        });
        self.script
            .create_subresource
            .next_or_unscripted(PORT, "create_subresource")
    }

    async fn patch_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        patch: &Patch,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.calls.record(ResourceCall::PatchSubresource {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            subresource: subresource.clone(),
            patch: patch.clone(),
            options: options.clone(),
        });
        self.script
            .patch_subresource
            .next_or_unscripted(PORT, "patch_subresource")
    }

    async fn replace_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
        body: &Value,
        options: &WriteOptions,
    ) -> OxiResult<Value> {
        self.calls.record(ResourceCall::ReplaceSubresource {
            kind: kind.clone(),
            namespace: owned(namespace),
            name: name.to_owned(),
            subresource: subresource.clone(),
            body: body.clone(),
            options: options.clone(),
        });
        self.script
            .replace_subresource
            .next_or_unscripted(PORT, "replace_subresource")
    }
}

/// `object` as a metadata-only feed delivers it: `apiVersion`, `kind` and `metadata`, marked
/// [partial](Resource::is_partial).
fn metadata_only(object: Resource) -> Resource {
    let json = serde_json::json!({
        "apiVersion": object.json["apiVersion"],
        "kind": object.json["kind"],
        "metadata": object.json["metadata"],
    });
    // The identity fields are copied from a valid `Resource`, so this cannot fail.
    Resource::from_json(json).map_or(object, Resource::into_partial)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::builders::pod;
    use futures::executor::block_on;
    use futures::{FutureExt, StreamExt};
    use oxikube_domain::ErrorKind;
    use oxikube_ports::ResourcePort;
    use serde_json::json;

    fn pods() -> Gvk {
        Gvk::new("", "v1", "Pod")
    }

    #[test]
    fn scripted_ok_and_err_are_returned_and_calls_recorded() {
        let fake = FakeResourcePort::new();
        let web = pod().name("web").running().build();
        fake.script()
            .get
            .push_ok(web.clone())
            .push_err(OxiError::forbidden("rbac"));
        let first = block_on(fake.get(&pods(), Some("demo"), "web")).unwrap();
        assert_eq!(first, web);
        let second = block_on(fake.get(&pods(), Some("demo"), "web")).unwrap_err();
        assert_eq!(second.kind(), ErrorKind::Forbidden);
        let get = ResourceCall::Get {
            kind: pods(),
            namespace: Some("demo".into()),
            name: "web".into(),
        };
        assert_eq!(fake.recorded_calls(), vec![get.clone(), get]);
        assert!(fake.mutating_calls().is_empty());
        fake.clear_calls();
        assert!(fake.recorded_calls().is_empty());
    }

    #[test]
    fn unscripted_reads_fall_back_to_the_store() {
        let a = pod().name("a").label("app", "web").running().build();
        let b = pod().name("b").label("app", "db").pending().build();
        let other_ns = pod().name("c").namespace("kube-system").build();
        let fake = FakeResourcePort::new().with_objects([a.clone(), b.clone(), other_ns.clone()]);

        let all = block_on(fake.list(&pods(), None, &ListOptions::default())).unwrap();
        assert_eq!(all.items.len(), 3);
        let demo = block_on(fake.list(&pods(), Some("demo"), &ListOptions::default())).unwrap();
        assert_eq!(demo.items, vec![a.clone(), b.clone()]);
        let web =
            block_on(fake.list(&pods(), None, &ListOptions::default().labels("app=web"))).unwrap();
        assert_eq!(web.items, vec![a.clone()]);
        let not_web = block_on(fake.list(
            &pods(),
            Some("demo"),
            &ListOptions::default().labels("app!=web"),
        ))
        .unwrap();
        assert_eq!(not_web.items, vec![b.clone()]);
        let by_name = block_on(fake.list(
            &pods(),
            None,
            &ListOptions::default().fields("metadata.name=c"),
        ))
        .unwrap();
        assert_eq!(by_name.items, vec![other_ns]);
        let meta =
            block_on(fake.list_metadata(&pods(), Some("demo"), &ListOptions::default())).unwrap();
        assert_eq!(meta.items, vec![a.meta.clone(), b.meta]);
        assert_eq!(block_on(fake.get(&pods(), Some("demo"), "a")).unwrap(), a);
        assert_eq!(
            block_on(fake.get(&pods(), Some("demo"), "zzz"))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            block_on(fake.get_opt(&pods(), Some("demo"), "zzz")).unwrap(),
            None
        );
        let unsupported = block_on(fake.list(
            &pods(),
            None,
            &ListOptions::default().fields("status.phase=Running"),
        ))
        .unwrap_err();
        assert_eq!(unsupported.kind(), ErrorKind::Unsupported);
        assert_eq!(
            block_on(fake.get_scale(&pods(), Some("demo"), "a"))
                .unwrap_err()
                .kind(),
            ErrorKind::Internal
        );
    }

    #[test]
    fn writes_update_the_store_and_are_recorded_as_mutations() {
        let fake = FakeResourcePort::new();
        let body = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "cfg"}});
        let cm = Gvk::new("", "v1", "ConfigMap");

        let dry =
            block_on(fake.create(&cm, Some("demo"), &body, &WriteOptions::dry_run())).unwrap();
        assert_eq!(dry.namespace(), Some("demo"));
        assert!(fake.objects().is_empty());

        block_on(fake.create(&cm, Some("demo"), &body, &WriteOptions::default())).unwrap();
        assert_eq!(fake.objects().len(), 1);
        let dup = block_on(fake.create(&cm, Some("demo"), &body, &WriteOptions::default()));
        assert_eq!(dup.unwrap_err().kind(), ErrorKind::Conflict);

        let replaced = json!({"apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "cfg", "namespace": "demo"}, "data": {"k": "v"}});
        let r = block_on(fake.replace(
            &cm,
            Some("demo"),
            "cfg",
            &replaced,
            &WriteOptions::default(),
        ))
        .unwrap();
        assert_eq!(r.get_str("/data/k"), Some("v"));

        let deleted =
            block_on(fake.delete(&cm, Some("demo"), "cfg", &DeleteOptions::default())).unwrap();
        assert_eq!(deleted, DeleteOutcome::Deleted);
        assert!(fake.objects().is_empty());
        assert_eq!(
            block_on(fake.delete(&cm, Some("demo"), "cfg", &DeleteOptions::default()))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );

        fake.script().scale.push_ok(Scale {
            replicas: 3,
            ..Scale::default()
        });
        let deploy = Gvk::new("apps", "v1", "Deployment");
        let scaled =
            block_on(fake.scale(&deploy, Some("demo"), "web", 3, &WriteOptions::default()))
                .unwrap();
        assert_eq!(scaled.replicas, 3);
        assert!(block_on(fake.evict("demo", "web", &DeleteOptions::dry_run())).is_err());

        let calls = fake.mutating_calls();
        assert_eq!(calls.len(), 8);
        assert!(calls[0].is_dry_run());
        assert!(!calls[1].is_dry_run());
        assert!(matches!(calls[7], ResourceCall::Evict { .. }));
        assert!(calls[7].is_dry_run());
    }

    #[test]
    fn port_is_usable_as_dyn_resource_port() {
        let fake: Arc<dyn ResourcePort> = Arc::new(FakeResourcePort::new());
        let reader: Arc<dyn oxikube_ports::ResourceReader> = fake;
        assert!(
            block_on(reader.get_opt(&pods(), None, "x"))
                .unwrap()
                .is_none()
        );
    }

    /// Watch replay: add at +0 s, modify at +1 s, delete at +5 s, delivered in order and
    /// only when the virtual clock reaches each offset.
    #[test]
    fn watch_replays_scripted_events_on_the_virtual_clock() {
        let fake = FakeResourcePort::new();
        let clock = fake.clock().clone();
        let start = clock.now();
        let added = pod().name("web").pending().build();
        let modified = pod().name("web").running().build();
        fake.script().watch.push_ok(
            Timeline::new()
                .ok_at(
                    Duration::ZERO,
                    DeltaBatch::from_deltas(vec![Delta::Applied(added.clone())]),
                )
                .ok_at(
                    Duration::from_secs(1),
                    DeltaBatch::from_deltas(vec![Delta::Applied(modified.clone())]),
                )
                .ok_at(
                    Duration::from_secs(5),
                    DeltaBatch::from_deltas(vec![Delta::Deleted(modified.clone())]),
                ),
        );
        let mut feed =
            block_on(fake.watch(&pods(), Some("demo"), &WatchOptions::default())).unwrap();

        let mut delivered = Vec::new();
        let take_ready = |feed: &mut WatchFeed<Resource>, delivered: &mut Vec<_>| {
            while let Some(Some(item)) = feed.next().now_or_never() {
                delivered.push((clock.now(), item.unwrap()));
            }
        };
        take_ready(&mut feed, &mut delivered);
        assert_eq!(delivered.len(), 1, "only the +0 s event is due");

        clock.advance(Duration::from_millis(999));
        take_ready(&mut feed, &mut delivered);
        assert_eq!(delivered.len(), 1, "+1 s event is not due at +0.999 s");

        clock.advance(Duration::from_millis(1));
        take_ready(&mut feed, &mut delivered);
        assert_eq!(delivered.len(), 2);

        clock.advance(Duration::from_secs(3));
        take_ready(&mut feed, &mut delivered);
        assert_eq!(delivered.len(), 2, "+5 s event is not due at +4 s");

        clock.advance(Duration::from_secs(1));
        take_ready(&mut feed, &mut delivered);
        assert_eq!(delivered.len(), 3);
        assert!(block_on(feed.next()).is_none(), "script ended");

        let at = |secs| start.checked_add(Duration::from_secs(secs)).unwrap();
        assert_eq!(
            delivered,
            vec![
                (at(0), DeltaBatch::from_deltas(vec![Delta::Applied(added)])),
                (
                    at(1),
                    DeltaBatch::from_deltas(vec![Delta::Applied(modified.clone())])
                ),
                (
                    at(5),
                    DeltaBatch::from_deltas(vec![Delta::Deleted(modified)])
                ),
            ]
        );
    }

    #[test]
    fn unscripted_watch_restarts_with_the_store_and_scripted_error_fails_the_call() {
        let a = pod().name("a").build();
        let fake = FakeResourcePort::new().with_objects([a.clone()]);
        fake.script()
            .watch
            .push_err(OxiError::forbidden("no watch"));
        let err = block_on(fake.watch(&pods(), None, &WatchOptions::default())).err();
        assert_eq!(err.map(|e| e.kind()), Some(ErrorKind::Forbidden));

        let mut feed = block_on(fake.watch(&pods(), None, &WatchOptions::default())).unwrap();
        let first = block_on(feed.next()).unwrap().unwrap();
        assert_eq!(first.deltas, vec![Delta::Restarted(vec![a])]);
        assert!(
            feed.next().now_or_never().is_none(),
            "stays open like a live watch"
        );
    }

    #[test]
    fn a_metadata_only_watch_delivers_partial_objects() {
        let a = pod().name("a").label("app", "web").build();
        assert!(a.get("/spec").is_some());
        let fake = FakeResourcePort::new().with_objects([a.clone()]);
        let options = WatchOptions::default().metadata_only();
        let mut feed = block_on(fake.watch(&pods(), None, &options)).unwrap();
        let first = block_on(feed.next()).unwrap().unwrap();
        let Delta::Restarted(listed) = &first.deltas[0] else {
            panic!("the first delta is the list");
        };
        assert!(listed[0].is_partial());
        assert_eq!(listed[0].meta, a.meta);
        assert_eq!(listed[0].kind, a.kind);
        assert!(listed[0].get("/spec").is_none() && listed[0].get("/status").is_none());
    }
}
