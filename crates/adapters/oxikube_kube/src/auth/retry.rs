//! "Retry once" for auth failures (the idea behind kdash's `should_retry_kubectl_refresh`,
//! without shelling out to `kubectl`).
//!
//! When a call fails with a retryable [`Auth`](oxikube_domain::ErrorKind::Auth) error the
//! pooled client is stale (an expired exec or OIDC token baked into it). The recovery is:
//! invalidate the pooled client, run the operation again (which obtains a freshly built
//! client), and report whatever that second attempt returns. It never loops: one
//! rebuild per failing call, so an auth outage cannot turn into a retry storm.

use std::future::Future;

use oxikube_domain::{ErrorKind, OxiError, OxiResult};

use super::classify::{CredentialRefresh, classify_with};

/// Runs `op`; if it fails with a retryable `Auth` error, awaits `invalidate` once and runs
/// `op` a second time.
///
/// `invalidate` drops or marks stale the pooled client for the context. `op` must fetch
/// its client from the pool on each call (so the second run sees the rebuilt one); it is
/// not handed a client here so this helper does not depend on the pool type.
///
/// The second attempt's result is final. If it fails with `Auth` too, the error is
/// returned with `retryable = false`: a refresh did not fix it, so the credential is
/// rejected and the session should go to `AuthRequired`. Errors of any other kind, and
/// non-retryable `Auth` errors, are returned immediately without invalidating.
pub async fn retry_once<T, Op, OpFut, Inv, InvFut>(invalidate: Inv, mut op: Op) -> OxiResult<T>
where
    Op: FnMut() -> OpFut,
    OpFut: Future<Output = OxiResult<T>>,
    Inv: FnOnce() -> InvFut,
    InvFut: Future<Output = ()>,
{
    match op().await {
        Err(err) if err.kind() == ErrorKind::Auth && err.is_retryable() => {
            invalidate().await;
            op().await.map_err(settle_after_retry)
        }
        other => other,
    }
}

/// [`retry_once`] for operations that fail with `kube::Error`: errors are classified with
/// [`classify_with`] before the retry decision.
pub async fn retry_once_kube<T, Op, OpFut, Inv, InvFut>(
    refresh: CredentialRefresh,
    invalidate: Inv,
    mut op: Op,
) -> OxiResult<T>
where
    Op: FnMut() -> OpFut,
    OpFut: Future<Output = Result<T, kube::Error>>,
    Inv: FnOnce() -> InvFut,
    InvFut: Future<Output = ()>,
{
    retry_once(invalidate, || {
        let fut = op();
        async move { fut.await.map_err(|e| classify_with(&e, refresh)) }
    })
    .await
}

/// After the one allowed retry, an `Auth` error is final.
fn settle_after_retry(err: OxiError) -> OxiError {
    if err.kind() == ErrorKind::Auth {
        err.with_retryable(false)
    } else {
        err
    }
}

#[cfg(test)]
mod tests;
