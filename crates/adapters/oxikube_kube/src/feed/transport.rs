//! The client a feed watch uses: the session's client, with watch responses uncompressed
//! and accepted watch requests counted.
//!
//! API servers that compress watch responses (seen on kind v1.37) flush every event as its
//! own gzip member. kube's `gzip` feature decodes through tower-http, whose multi-member
//! support intermittently fails mid-stream ("there are extra bytes after body has been
//! decompressed"), which would show up as spurious feed errors and, during a streaming list,
//! restart the initial list. Watch events are small and incremental, so feeds ask for
//! `Accept-Encoding: identity` on watch requests only; paged `LIST` responses (one gzip
//! member each) stay compressed. tower-http's decompression layer keeps a caller's
//! `Accept-Encoding`, so the header set here reaches the server.
//!
//! kube's watcher reports nothing when a watch request succeeds, only the events that
//! follow, so a reconnected but quiet watch looks the same as one still backing off or
//! connecting. The client counts every watch request the server accepts (2xx) on a
//! [`Accepted`] channel; the watch uses it to leave
//! [`FeedState::Retrying`](super::FeedState::Retrying) only once a reconnect got through.

use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::header::{ACCEPT_ENCODING, HeaderValue};
use http::{Request, Response, Uri};
use kube::Client;
use kube::client::Body;
use tokio::sync::watch;
use tower::Service;

/// How many watch requests the server has accepted (2xx) so far; changes on every one.
pub(super) type Accepted = watch::Receiver<u64>;

/// `client` with `Accept-Encoding: identity` on every watch request, and the count of
/// accepted watch requests made through it. Shares `client`'s connection pool and auth;
/// cheap to build, so each feed watch gets its own.
pub(super) fn watch_client(client: &Client) -> (Client, Accepted) {
    let (accepted, rx) = watch::channel(0);
    let service = IdentityWatches {
        client: client.clone(),
        accepted,
    };
    (Client::new(service, client.default_namespace()), rx)
}

/// The service behind [`watch_client`]: sets the header, sends through the inner client and
/// counts accepted watches.
#[derive(Clone)]
struct IdentityWatches {
    client: Client,
    accepted: watch::Sender<u64>,
}

impl Service<Request<Body>> for IdentityWatches {
    type Response = Response<Body>;
    type Error = kube::Error;
    type Future = BoxFuture<'static, Result<Response<Body>, kube::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        let watch = is_watch(request.uri());
        if watch {
            request
                .headers_mut()
                .insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        }
        let inner = self.client.clone();
        let accepted = self.accepted.clone();
        Box::pin(async move {
            let response = inner.send(request).await?;
            // Counted before the watcher sees the response, so before any event or error
            // of this watch reaches the feed.
            if watch && response.status().is_success() {
                accepted.send_modify(|n| *n = n.wrapping_add(1));
            }
            Ok(response)
        })
    }
}

/// Whether `uri` is a watch request (`watch=true`, or the `watch=1` spelling).
fn is_watch(uri: &Uri) -> bool {
    uri.query().is_some_and(|query| {
        query
            .split('&')
            .any(|pair| pair == "watch=true" || pair == "watch=1")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_watch_requests_are_recognised() {
        let uri = |s: &str| s.parse::<Uri>().unwrap();
        assert!(is_watch(&uri("/api/v1/pods?watch=true&resourceVersion=1")));
        assert!(is_watch(&uri("/api/v1/pods?limit=5&watch=1")));
        assert!(!is_watch(&uri("/api/v1/pods?limit=500")));
        assert!(!is_watch(&uri("/api/v1/pods")));
        assert!(!is_watch(&uri("/api/v1/pods?watch=false")));
    }
}
