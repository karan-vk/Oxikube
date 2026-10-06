//! A point-in-time view of the catalog, and the diff between two of them.
//!
//! The port's [`ClusterContext`] carries only name, source, server and namespace. A change to a
//! context's user, its credentials or the kubeconfig's `current-context` must still reach the
//! pool and the UI, so each entry also carries a [`Fingerprint`]: a SHA-256 over the pool's
//! [`ContextDefinition`] of the context (its context, cluster and user entries as parsed), so
//! the UI's diff and the pool's invalidation share one definition of a context's connection. Two snapshots with the same digest are
//! identical, which is what makes a `touch` with no content change emit nothing.
//!
//! Only digests are kept, never the credentials they were computed from. Files a kubeconfig
//! merely points at (`certificate-authority`, `token-file`, ...) are not hashed: rotating one in
//! place is not a catalog change.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ClusterSource, SourceId, SourceStatus, SourcesChanged};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::layout::Layout;
use super::status;
use crate::kubeconfig::{Diagnostic, LoadedKubeconfig};
use crate::pool::ContextDefinition;

/// SHA-256 of everything that makes a catalog entry what it is.
type Fingerprint = [u8; 32];

/// One catalog entry: what the port shows, plus the fingerprint used for diffing.
struct Entry {
    context: ClusterContext,
    fingerprint: Fingerprint,
}

/// The catalog as of one load.
pub(super) struct Snapshot {
    pub(super) sources: Vec<ClusterSource>,
    /// How reading each source went, in source order.
    pub(super) statuses: Vec<SourceStatus>,
    entries: Vec<Entry>,
    pub(super) current_context: Option<ContextName>,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) loaded: Arc<LoadedKubeconfig>,
    digest: [u8; 32],
}

impl Snapshot {
    /// Build the snapshot of a finished load. `layout` says which source owns each file.
    pub(super) fn build(loaded: LoadedKubeconfig, layout: Layout) -> Self {
        let mut entries = Vec::new();
        for name in loaded.context_names() {
            let (Some(cluster), Some(origin)) = (loaded.cluster_id(&name), loaded.origin(&name))
            else {
                continue;
            };
            // Every origin has an owner: files come from the layout, pasted text and the
            // in-cluster context are registered as virtual entries. The fallback only keeps a
            // future source that forgot to register visible instead of dropping its contexts.
            let source = layout
                .owner_of(origin)
                .cloned()
                .unwrap_or_else(|| SourceId(origin.display().to_string()));
            let (context, fingerprint) = describe(&loaded, name, cluster, source);
            entries.push(Entry {
                context,
                fingerprint,
            });
        }
        let current_context = loaded
            .merged
            .current_context
            .as_deref()
            .map(ContextName::new);
        let mut digest = Sha256::new();
        for entry in &entries {
            put(&mut digest, entry.context.cluster.as_str().as_bytes());
            put(&mut digest, &entry.fingerprint);
        }
        put(
            &mut digest,
            current_context
                .as_ref()
                .map_or("", |c| c.as_str())
                .as_bytes(),
        );
        let diagnostics: Vec<Diagnostic> = layout
            .diagnostics
            .iter()
            .chain(&loaded.diagnostics)
            .cloned()
            .collect();
        let mut owned: HashMap<SourceId, usize> = HashMap::new();
        for entry in &entries {
            *owned.entry(entry.context.source.clone()).or_default() += 1;
        }
        let statuses = status::build(&layout, &loaded.sources, &diagnostics, &owned);
        Snapshot {
            sources: layout.sources(),
            statuses,
            entries,
            current_context,
            diagnostics,
            loaded: Arc::new(loaded),
            digest: digest.finalize().into(),
        }
    }

    /// The catalog entries as the port reports them.
    pub(super) fn contexts(&self) -> Vec<ClusterContext> {
        self.entries.iter().map(|e| e.context.clone()).collect()
    }

    /// What changed from `old` to `self`. `None` means nothing was loaded before.
    ///
    /// An entry is `changed` when its fingerprint differs (source, server, namespace, user or
    /// credentials) or when it gained or lost `current-context`: the port has no field for
    /// that, so both contexts are reported as changed and the UI reads
    /// [`current_context`](super::KubeconfigSources::current_context).
    pub(super) fn diff_from(&self, old: Option<&Snapshot>) -> SourcesChanged {
        let Some(old) = old else {
            return SourcesChanged {
                added: self.contexts(),
                ..SourcesChanged::default()
            };
        };
        if old.digest == self.digest {
            return SourcesChanged::default();
        }
        let before: HashMap<&ClusterId, &Entry> = old
            .entries
            .iter()
            .map(|e| (&e.context.cluster, e))
            .collect();
        let after: HashMap<&ClusterId, &Entry> = self
            .entries
            .iter()
            .map(|e| (&e.context.cluster, e))
            .collect();
        let current_flipped = old.current_context != self.current_context;
        let touches_current = |name: &ContextName| {
            current_flipped
                && (old.current_context.as_ref() == Some(name)
                    || self.current_context.as_ref() == Some(name))
        };
        let mut diff = SourcesChanged::default();
        for entry in &self.entries {
            match before.get(&entry.context.cluster) {
                None => diff.added.push(entry.context.clone()),
                Some(previous)
                    if previous.fingerprint != entry.fingerprint
                        || touches_current(&entry.context.context) =>
                {
                    diff.changed.push(entry.context.clone());
                }
                Some(_) => {}
            }
        }
        for entry in &old.entries {
            if !after.contains_key(&entry.context.cluster) {
                diff.removed.push(entry.context.cluster.clone());
            }
        }
        diff
    }
}

/// The port view of one context and its fingerprint.
///
/// The fingerprint covers the pool's [`ContextDefinition`] of the context (its context,
/// cluster and user entries, and whether it is the synthetic in-cluster context), so the
/// diff reports a context as `changed` exactly when
/// [`ClientPool::replace_loaded`](crate::ClientPool::replace_loaded) drops its client, plus
/// the owning source, which the pool does not track.
fn describe(
    loaded: &LoadedKubeconfig,
    name: ContextName,
    cluster: ClusterId,
    source: SourceId,
) -> (ClusterContext, Fingerprint) {
    let definition = ContextDefinition::from_kubeconfig(&loaded.merged, &name)
        .map(|d| d.with_in_cluster(loaded.is_in_cluster(&name)));
    let slice = definition.as_ref().map(ContextDefinition::kubeconfig);
    let server = slice
        .and_then(|k| k.clusters.first())
        .and_then(|c| c.cluster.as_ref())
        .and_then(|c| c.server.clone());
    let default_namespace = slice
        .and_then(|k| k.contexts.first())
        .and_then(|c| c.context.as_ref())
        .and_then(|c| c.namespace.clone());

    let mut hasher = Sha256::new();
    put(&mut hasher, source.0.as_bytes());
    hasher.update([u8::from(
        definition
            .as_ref()
            .is_some_and(ContextDefinition::is_in_cluster),
    )]);
    // The slice serialises to JSON (credentials included, in memory only) and only the digest
    // leaves this function. A serialisation failure hashes as `null`: the entry then compares
    // equal to any other failing entry, which can only hide a change, never invent one.
    let value = slice.and_then(|k| serde_json::to_value(k).ok());
    hash_value(&mut hasher, &value.unwrap_or(Value::Null));
    let references = references(
        slice,
        definition
            .as_ref()
            .is_some_and(ContextDefinition::is_in_cluster),
    );
    let context = ClusterContext {
        server,
        default_namespace,
        cluster_name: references.cluster,
        user: references.user,
        problem: references.problem,
        ..ClusterContext::new(cluster, name, source)
    };
    (context, hasher.finalize().into())
}

/// The cluster and user a context names, and what is wrong with them.
struct References {
    cluster: Option<String>,
    user: Option<String>,
    problem: Option<String>,
}

/// Reads the names a context points at out of its one-context `slice` and reports a name the
/// kubeconfig does not define. The in-cluster context names neither and is never a problem.
/// Only names leave this function, never the entries behind them.
fn references(slice: Option<&kube::config::Kubeconfig>, in_cluster: bool) -> References {
    let named = slice
        .and_then(|k| k.contexts.first())
        .and_then(|c| c.context.as_ref());
    let cluster = named
        .map(|c| c.cluster.clone())
        .filter(|name| !name.is_empty());
    let user = named.and_then(|c| c.user.clone()).filter(|u| !u.is_empty());
    let problem = if in_cluster {
        None
    } else if named.is_none() {
        Some("the context has no cluster or user".to_owned())
    } else if cluster.is_none() {
        Some("the context names no cluster".to_owned())
    } else if slice.is_some_and(|k| k.clusters.is_empty()) {
        cluster
            .as_deref()
            .map(|name| format!("cluster \"{name}\" is not defined in the kubeconfig"))
    } else if user.is_some() && slice.is_some_and(|k| k.auth_infos.is_empty()) {
        user.as_deref()
            .map(|name| format!("user \"{name}\" is not defined in the kubeconfig"))
    } else {
        None
    };
    References {
        cluster,
        user,
        problem,
    }
}

/// Length-prefix `bytes` into `hasher` so adjacent fields cannot run together.
fn put(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Hash a JSON value with object keys sorted.
///
/// kube's `AuthInfo` and `ExecConfig` hold `HashMap`s whose iteration order differs from one
/// parse to the next; hashing the serialised bytes directly would report a change on every
/// reload of an unchanged file.
fn hash_value(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update([0]),
        Value::Bool(b) => hasher.update([1, u8::from(*b)]),
        Value::Number(n) => {
            hasher.update([2]);
            put(hasher, n.to_string().as_bytes());
        }
        Value::String(s) => {
            hasher.update([3]);
            put(hasher, s.as_bytes());
        }
        Value::Array(items) => {
            hasher.update([4]);
            hasher.update((items.len() as u64).to_le_bytes());
            for item in items {
                hash_value(hasher, item);
            }
        }
        Value::Object(map) => {
            hasher.update([5]);
            hasher.update((map.len() as u64).to_le_bytes());
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                put(hasher, key.as_bytes());
                hash_value(hasher, &map[key]);
            }
        }
    }
}
