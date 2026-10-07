//! A deadline on credential refreshes inside a live client.
//!
//! kube refreshes an expiring exec-plugin token (or GCP command / OIDC token) inside the
//! client's auth layer, on the request path, while holding the token mutex and with no
//! timeout. A plugin that hangs there queues every request on that client behind it.
//!
//! [`RefreshGuardLayer`] takes the auth layer out of kube's stack and runs it in front of
//! the stack instead, against a stand-in service that only hands the request back
//! ([`Echo`]). That splits "get the `Authorization` header" from "send the request", so
//! only the first half is bounded:
//!
//! * The authorize step has a deadline. On expiry the request fails with
//!   [`RefreshStalled`], which classifies as a retryable `Auth` error: callers that retry
//!   once (`retry_once`) invalidate the pooled client and rebuild it.
//! * The refresh is never cancelled and never duplicated. A plugin run cannot be
//!   cancelled (`std::process::Command::output` on a blocking thread), and dropping
//!   kube's future would release the token mutex so the next request would start another
//!   process. The unfinished future is handed to a detached task instead, which keeps the
//!   token mutex until the plugin returns and kube has cached the token.
//! * Only a refresh that passed the deadline marks the client stalled: while such a task is
//!   alive new requests fail fast with [`RefreshStalled`] rather than queueing behind the
//!   plugin, and the client recovers without a rebuild once the plugin returns. A caller
//!   that merely cancels (a dropped LIST, a watch restart) while the refresh is still
//!   inside the deadline does not: the next request queues on the detached refresh's
//!   token mutex under its own deadline, so a healthy refresh is neither failed nor
//!   rebuilt around.
//! * Requests that already have a token pay one `Mutex` hop and one timer; sending the
//!   request is not bounded, so streams and slow LISTs are unaffected.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::{BoxFuture, Ready, ready};
use http::{Request, Response};
use kube::client::Body;
use kube::client::middleware::AuthLayer;
use tower::{BoxError, Layer, Service, ServiceExt as _};

/// Default limit on one credential refresh inside a live client. The same figure as
/// `DEFAULT_EXEC_DEADLINE`: long enough for a slow token exchange, short enough that a
/// hung plugin shows up as an error.
pub const DEFAULT_REFRESH_DEADLINE: Duration = Duration::from_secs(30);

/// Why a request could not get its credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStalled {
    /// This request waited for the refresh up to the deadline and gave up.
    TimedOut(Duration),
    /// An earlier refresh has not finished; the request was not queued behind it.
    InFlight,
}

impl fmt::Display for RefreshStalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut(d) => write!(
                f,
                "the credential refresh did not finish within {}",
                super::describe(*d)
            ),
            Self::InFlight => f.write_str("a credential refresh is still running"),
        }
    }
}

impl std::error::Error for RefreshStalled {}

/// Hands the request back, as the "response" body (tower-http's `AddAuthorization` wants a
/// `Response`), so the auth layer's header lands on it.
#[derive(Debug, Clone, Copy)]
struct Echo;

impl Service<Request<Body>> for Echo {
    type Response = Response<Request<Body>>;
    type Error = BoxError;
    type Future = Ready<Result<Response<Request<Body>>, BoxError>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        ready(Ok(Response::new(request)))
    }
}

type Authorize = <AuthLayer as Layer<Echo>>::Service;
type Pending = BoxFuture<'static, Result<Response<Request<Body>>, BoxError>>;

/// Moves kube's auth layer in front of the client stack, bounded by a deadline. See the
/// module docs.
pub struct RefreshGuardLayer {
    gate: Gate,
}

impl RefreshGuardLayer {
    /// Wraps `auth` (from `Config::auth_layer`); the client built from the same config must
    /// have no credentials of its own left, or the header is set twice.
    pub fn new(auth: &AuthLayer, deadline: Duration) -> Self {
        Self {
            gate: Gate {
                authorize: auth.layer(Echo),
                deadline,
                stalled: Arc::default(),
            },
        }
    }
}

impl<S> Layer<S> for RefreshGuardLayer {
    type Service = RefreshGuard<S>;

    fn layer(&self, inner: S) -> RefreshGuard<S> {
        RefreshGuard {
            inner: Arc::new(tokio::sync::Mutex::new(inner)),
            gate: self.gate.clone(),
        }
    }
}

/// The service [`RefreshGuardLayer`] builds.
pub struct RefreshGuard<S> {
    inner: Arc<tokio::sync::Mutex<S>>,
    gate: Gate,
}

impl<S> Service<Request<Body>> for RefreshGuard<S>
where
    S: Service<Request<Body>, Error = BoxError> + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<S::Response, BoxError>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        // Readiness of the inner stack is awaited in `call`, after the credential.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let gate = self.gate.clone();
        let inner = self.inner.clone();
        Box::pin(async move {
            let request = gate.authorize(request).await?;
            let future = {
                let mut inner = inner.lock().await;
                inner.ready().await?;
                inner.call(request)
            };
            future.await
        })
    }
}

#[derive(Clone)]
struct Gate {
    authorize: Authorize,
    deadline: Duration,
    /// Refreshes handed to a detached task that have not finished.
    stalled: Arc<AtomicUsize>,
}

impl Gate {
    async fn authorize(&self, request: Request<Body>) -> Result<Request<Body>, BoxError> {
        if self.stalled.load(Ordering::Acquire) > 0 {
            return Err(RefreshStalled::InFlight.into());
        }
        let mut authorize = self.authorize.clone();
        authorize.ready().await?;
        let mut refresh = Refresh {
            future: Some(Box::pin(authorize.call(request))),
            stalled: self.stalled.clone(),
            timed_out: false,
        };
        let pending = refresh.future.as_mut().expect("just set");
        match tokio::time::timeout(self.deadline, pending).await {
            Ok(result) => {
                refresh.future = None;
                result.map(Response::into_body)
            }
            // `refresh` drops at the end of this arm and takes the future with it.
            Err(_) => {
                refresh.timed_out = true;
                Err(RefreshStalled::TimedOut(self.deadline).into())
            }
        }
    }
}

/// An unfinished credential refresh. Dropping it before it completes (the deadline passed,
/// or the caller went away) lets the future run to the end in a detached task, so the
/// refresh finishes once and the token mutex is not released early. Only a refresh that
/// passed the deadline (`timed_out`) counts as stalled; a cancelled one still inside the
/// deadline is detached without failing later requests fast.
struct Refresh {
    future: Option<Pending>,
    stalled: Arc<AtomicUsize>,
    timed_out: bool,
}

impl Drop for Refresh {
    fn drop(&mut self) {
        let Some(future) = self.future.take() else {
            return;
        };
        // No runtime (the app is shutting down): nothing to finish, the future is dropped.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let stalled = self.timed_out.then(|| {
            self.stalled.fetch_add(1, Ordering::AcqRel);
            self.stalled.clone()
        });
        runtime.spawn(async move {
            // The outcome is not wanted; running it updates kube's cached token.
            let _ = future.await;
            if let Some(stalled) = stalled {
                stalled.fetch_sub(1, Ordering::AcqRel);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_messages_name_the_deadline() {
        let timed_out = RefreshStalled::TimedOut(Duration::from_secs(30)).to_string();
        assert!(timed_out.contains("30s"), "{timed_out}");
        assert!(
            RefreshStalled::InFlight
                .to_string()
                .contains("still running")
        );
    }

    fn refresh(timed_out: bool, stalled: &Arc<AtomicUsize>) -> Refresh {
        Refresh {
            future: Some(Box::pin(std::future::pending())),
            stalled: stalled.clone(),
            timed_out,
        }
    }

    #[test]
    fn dropping_a_refresh_outside_a_runtime_is_harmless() {
        // App shutdown: the runtime is gone, so there is nothing to detach to and the client
        // must not be left marked stalled.
        let stalled = Arc::new(AtomicUsize::new(0));
        drop(refresh(true, &stalled));
        drop(refresh(false, &stalled));
        assert_eq!(stalled.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn only_a_refresh_past_the_deadline_marks_the_client_stalled() {
        let stalled = Arc::new(AtomicUsize::new(0));
        // Cancelled while still inside the deadline: detached, but later requests are not
        // failed fast.
        drop(refresh(false, &stalled));
        assert_eq!(stalled.load(Ordering::Acquire), 0);
        // Past the deadline: stalled until the detached future ends (this one never does).
        drop(refresh(true, &stalled));
        assert_eq!(stalled.load(Ordering::Acquire), 1);
    }
}
