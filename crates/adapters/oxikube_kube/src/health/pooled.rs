//! Health probes through the [`ClientPool`]: the glue between `get` and the probes.
//!
//! Each call fetches the client from the pool, so a probe after
//! [`invalidate`](ClientPool::invalidate) sees a freshly built client. A failed probe whose
//! error is a retryable `Auth` (an expired exec or OIDC token baked into the pooled client)
//! invalidates the entry and runs once more, through [`retry_once`]; a second `Auth`
//! failure is final and non-retryable, which the liveness policy treats as permanent.

use std::future::Future;
use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ContextName;

use super::capabilities::CapabilityReport;
use super::liveness::probe_apiserver_version;
use super::rules::{RulesCache, probe_capabilities};
use crate::auth::{CredentialRefresh, retry_once};
use crate::pool::ClientPool;

/// `GET /version` for `context` using the pooled client, rebuilding it once after a
/// retryable auth failure. Returns the server's `gitVersion`.
///
/// # Errors
///
/// `NotFound` for an unknown context, the pool's build error, or the classified probe
/// failure.
pub async fn probe_context(
    pool: &ClientPool,
    context: &ContextName,
    refresh: CredentialRefresh,
) -> OxiResult<String> {
    retry_once(
        || async {
            pool.invalidate(context);
        },
        || async {
            let client = pool.get(context).await?;
            probe_apiserver_version(&client, refresh).await
        },
    )
    .await
}

/// RBAC capability levels for `namespace` in `context`, using the pooled client and
/// `cache`, rebuilding the client once after a retryable auth failure.
///
/// # Errors
///
/// As [`probe_context`], plus the review request's classified failure.
pub async fn capabilities_for_context(
    pool: &ClientPool,
    cache: &RulesCache,
    context: &ContextName,
    namespace: &str,
    refresh: CredentialRefresh,
) -> OxiResult<CapabilityReport> {
    retry_once(
        || async {
            pool.invalidate(context);
        },
        || async {
            let client = pool.get(context).await?;
            probe_capabilities(&client, cache, context, namespace, refresh).await
        },
    )
    .await
}

/// A probe closure for [`Liveness::spawn`](super::Liveness::spawn) that checks `context`
/// through `pool` on every call.
pub fn pooled_probe(
    pool: Arc<ClientPool>,
    context: ContextName,
    refresh: CredentialRefresh,
) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = OxiResult<String>> + Send>> + Send + 'static
{
    move || {
        let pool = pool.clone();
        let context = context.clone();
        Box::pin(async move { probe_context(&pool, &context, refresh).await })
    }
}

#[cfg(test)]
mod tests {
    use kube::config::Kubeconfig;
    use oxikube_domain::ErrorKind;

    use super::*;
    use crate::pool::PoolConfig;

    fn empty_pool() -> ClientPool {
        let kubeconfig: Kubeconfig = serde_json::from_str("{}").unwrap();
        ClientPool::new(kubeconfig, PoolConfig::default())
    }

    #[tokio::test]
    async fn unknown_context_is_not_found_and_not_retried() {
        let pool = empty_pool();
        let err = probe_context(&pool, &ContextName::new("nope"), CredentialRefresh::Unknown)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = capabilities_for_context(
            &pool,
            &RulesCache::default(),
            &ContextName::new("nope"),
            "default",
            CredentialRefresh::Unknown,
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn pooled_probe_reports_the_pool_error() {
        let mut probe = pooled_probe(
            Arc::new(empty_pool()),
            ContextName::new("nope"),
            CredentialRefresh::Unknown,
        );
        assert_eq!(probe().await.unwrap_err().kind(), ErrorKind::NotFound);
    }
}
