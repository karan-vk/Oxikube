//! Pool entries and the per-context slice of the kubeconfig they were built from.
//!
//! Both types hold credentials (tokens, key data, exec config), so their `Debug`
//! is written by hand and prints only the context name and the server host.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use kube::Client;
use kube::config::Kubeconfig;
use oxikube_domain::ids::ContextName;
use tokio::sync::OnceCell;

/// The part of a merged kubeconfig one context depends on: the context entry, its
/// cluster and its user, as a single-context [`Kubeconfig`] whose
/// `current-context` is that context.
///
/// The pool builds a client from this slice and compares slices to decide which
/// clients a new kubeconfig invalidates.
#[derive(Clone)]
pub struct ContextDefinition {
    context: ContextName,
    kubeconfig: Kubeconfig,
}

impl ContextDefinition {
    /// Slices `context` out of `kubeconfig`. `None` when the context is not defined.
    ///
    /// A missing cluster or user is kept missing, so building reports kube's own
    /// error for it. Like kube, the first entry wins when a name repeats.
    pub fn from_kubeconfig(kubeconfig: &Kubeconfig, context: &ContextName) -> Option<Self> {
        let named_context = kubeconfig
            .contexts
            .iter()
            .find(|c| c.name == context.as_str())?;
        let ctx = named_context.context.as_ref();
        let cluster = ctx.and_then(|c| {
            kubeconfig
                .clusters
                .iter()
                .find(|cl| cl.name == c.cluster)
                .cloned()
        });
        let user = ctx
            .and_then(|c| c.user.as_ref())
            .and_then(|u| kubeconfig.auth_infos.iter().find(|a| &a.name == u).cloned());
        Some(Self {
            context: context.clone(),
            kubeconfig: Kubeconfig {
                current_context: Some(context.as_str().to_owned()),
                contexts: vec![named_context.clone()],
                clusters: cluster.into_iter().collect(),
                auth_infos: user.into_iter().collect(),
                ..Kubeconfig::default()
            },
        })
    }

    /// The context this definition is for.
    pub fn context(&self) -> &ContextName {
        &self.context
    }

    /// The single-context kubeconfig. Holds credentials; never log it.
    pub fn kubeconfig(&self) -> &Kubeconfig {
        &self.kubeconfig
    }

    /// Host of the cluster's `server` URL, when it parses. Safe to log.
    pub fn server_host(&self) -> Option<String> {
        let server = self
            .kubeconfig
            .clusters
            .first()?
            .cluster
            .as_ref()?
            .server
            .as_ref()?;
        url::Url::parse(server).ok()?.host_str().map(str::to_owned)
    }

    /// Whether `other` describes the same connection: same context, cluster and
    /// user entries, credentials included (a rotated token counts as a change).
    pub fn same_connection(&self, other: &Self) -> bool {
        // Compared through serde because kube's kubeconfig types only implement
        // `PartialEq` under `cfg(test)`. Secret fields serialise in clear inside
        // this transient value; it is never stored or logged. A serialisation
        // failure counts as "changed", which only costs a rebuild.
        match (
            serde_json::to_value(&self.kubeconfig),
            serde_json::to_value(&other.kubeconfig),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

impl fmt::Debug for ContextDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextDefinition")
            .field("context", &self.context.as_str())
            .field("server_host", &self.server_host())
            .finish_non_exhaustive()
    }
}

/// One cached context: its definition and the client, built at most once.
pub(crate) struct PoolEntry {
    pub(crate) definition: Arc<ContextDefinition>,
    /// Filled by the first successful build. Callers clone this `Arc` while they
    /// build or read, which also marks the entry as in use for eviction.
    pub(crate) cell: Arc<OnceCell<Arc<Client>>>,
    pub(crate) last_used: Instant,
}

impl PoolEntry {
    pub(crate) fn new(definition: ContextDefinition, now: Instant) -> Self {
        Self {
            definition: Arc::new(definition),
            cell: Arc::new(OnceCell::new()),
            last_used: now,
        }
    }

    /// Referenced outside the pool: a build is running, a caller is mid-`get`, or
    /// a caller holds the client.
    pub(crate) fn is_referenced(&self) -> bool {
        Arc::strong_count(&self.cell) > 1
            || self
                .cell
                .get()
                .is_some_and(|client| Arc::strong_count(client) > 1)
    }
}

impl fmt::Debug for PoolEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolEntry")
            .field("context", &self.definition.context.as_str())
            .field("server_host", &self.definition.server_host())
            .field("built", &self.cell.initialized())
            .finish_non_exhaustive()
    }
}
