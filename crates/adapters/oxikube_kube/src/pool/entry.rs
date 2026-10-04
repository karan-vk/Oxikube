//! Pool entries and the per-context slice of the kubeconfig they were built from.
//!
//! Both types hold credentials (tokens, key data, exec config), so their `Debug`
//! is written by hand and prints only the context name and the server host.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kube::Client;
use kube::config::Kubeconfig;
use oxikube_domain::OxiError;
use oxikube_domain::ids::ContextName;
use parking_lot::Mutex;
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;

use crate::auth::describe;

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
    in_cluster: bool,
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
            in_cluster: false,
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

    /// Marks this definition as the synthetic in-cluster context (E03-S10), by provenance,
    /// so [`build_config`](super::build_config) applies the in-cluster client fix-ups.
    pub fn with_in_cluster(mut self, in_cluster: bool) -> Self {
        self.in_cluster = in_cluster;
        self
    }

    /// Whether this is the synthetic in-cluster context (not merely a context of that name).
    pub fn is_in_cluster(&self) -> bool {
        self.in_cluster
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
        if self.in_cluster != other.in_cluster {
            return false;
        }
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

/// What `get` needs from an entry: its cell, definition and in-flight build slot.
/// Holding the cell `Arc` marks the entry in use until the caller drops it.
pub(crate) type Checkout = (
    Arc<OnceCell<Arc<Client>>>,
    Arc<ContextDefinition>,
    Arc<PendingBuild>,
);

/// One cached context: its definition and the client, built at most once.
pub(crate) struct PoolEntry {
    pub(crate) definition: Arc<ContextDefinition>,
    /// Filled by the first successful build. Callers clone this `Arc` while they
    /// build or read, which also marks the entry as in use for eviction.
    pub(crate) cell: Arc<OnceCell<Arc<Client>>>,
    /// A build that outlived its deadline, kept so the next `get` resumes it.
    pub(crate) pending: Arc<PendingBuild>,
    pub(crate) last_used: Instant,
}

impl PoolEntry {
    pub(crate) fn new(definition: ContextDefinition, now: Instant) -> Self {
        Self {
            definition: Arc::new(definition),
            cell: Arc::new(OnceCell::new()),
            pending: Arc::new(PendingBuild::default()),
            last_used: now,
        }
    }

    /// The handles `get` needs.
    pub(crate) fn checkout(&self) -> Checkout {
        (
            self.cell.clone(),
            self.definition.clone(),
            self.pending.clone(),
        )
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

type BuildHandle = JoinHandle<Result<Client, OxiError>>;

/// The blocking build of one entry that is still running after its caller gave up.
///
/// A build cannot be cancelled: an exec plugin process runs to completion on the
/// blocking pool. Without this slot every `get` after a timeout (or after a
/// cancelled `get`) would start another plugin process next to the hung one.
/// Instead the abandoned task's handle is parked here and the next `get` awaits it
/// again, under a fresh deadline. A build that finished meanwhile is used as is. The
/// slot dies with its entry, so invalidation never resumes a stale build.
#[derive(Default)]
pub(crate) struct PendingBuild(Mutex<Option<BuildHandle>>);

impl PendingBuild {
    /// Resumes the parked build, or starts one with `start`, and waits at most
    /// `deadline` for it.
    pub(crate) async fn run(
        &self,
        context: &ContextName,
        deadline: Duration,
        start: impl FnOnce() -> BuildHandle,
    ) -> Result<Client, OxiError> {
        let handle = self.0.lock().take().unwrap_or_else(start);
        let mut parked = Parked {
            slot: self,
            handle: Some(handle),
        };
        let outcome = match parked.handle.as_mut() {
            Some(handle) => tokio::time::timeout(deadline, handle).await,
            // Just set above.
            None => return Err(OxiError::internal("client build handle missing")),
        };
        match outcome {
            Ok(joined) => {
                // The task finished; a finished handle must not be polled again.
                parked.handle = None;
                joined.map_err(|join| {
                    let what = if join.is_panic() {
                        "panicked"
                    } else {
                        "was cancelled"
                    };
                    OxiError::internal(format!("building the client for `{context}` {what}"))
                })?
            }
            // `parked` puts the handle back on drop, for the next `get`.
            Err(_elapsed) => Err(OxiError::timeout(format!(
                "context `{context}`: building the client (exec credential plugin) did not \
                 finish within {}",
                describe(deadline)
            ))),
        }
    }
}

/// Puts an unfinished build back into its slot when the waiter stops waiting:
/// on timeout, and also when the `get` future is dropped mid-wait.
struct Parked<'a> {
    slot: &'a PendingBuild,
    handle: Option<BuildHandle>,
}

impl Drop for Parked<'_> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            *self.slot.0.lock() = Some(handle);
        }
    }
}

// `run` is awaited inside `OnceCell::get_or_try_init` from `ClientPool::get`, whose
// future must stay `Send`.
const _: fn() = || {
    fn assert_send<F: Future + Send>(_: F) {}
    let pending = PendingBuild::default();
    let context = ContextName::from("x");
    assert_send(pending.run(&context, Duration::ZERO, || {
        tokio::task::spawn_blocking(|| Err(OxiError::internal("x")))
    }));
};
