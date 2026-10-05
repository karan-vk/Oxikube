//! The client a feed watches with: the session's client, with watch responses uncompressed.
//!
//! API servers that compress watch responses (seen on kind v1.37) flush every event as its
//! own gzip member. kube's `gzip` feature decodes through tower-http, whose multi-member
//! support intermittently fails mid-stream ("there are extra bytes after body has been
//! decompressed"), which would show up as spurious feed errors and, during a streaming list,
//! restart the initial list. Watch events are small and incremental, so feeds ask for
//! `Accept-Encoding: identity` on watch requests only; paged `LIST` responses (one gzip
//! member each) stay compressed. tower-http's decompression layer keeps a caller's
//! `Accept-Encoding`, so the header set here reaches the server.

use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::header::{ACCEPT_ENCODING, HeaderValue};
use http::{Request, Response, Uri};
use kube::Client;
use kube::client::Body;
use tower::Service;

/// `client` with `Accept-Encoding: identity` on every watch request. Shares `client`'s
/// connection pool and auth; cheap to build.
pub(super) fn watch_client(client: &Client) -> Client {
    Client::new(IdentityWatches(client.clone()), client.default_namespace())
}

/// The service behind [`watch_client`]: sets the header, then sends through the inner client.
#[derive(Clone)]
struct IdentityWatches(Client);

impl Service<Request<Body>> for IdentityWatches {
    type Response = Response<Body>;
    type Error = kube::Error;
    type Future = BoxFuture<'static, Result<Response<Body>, kube::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request<Body>) -> Self::Future {
        if is_watch(request.uri()) {
            request
                .headers_mut()
                .insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        }
        let inner = self.0.clone();
        Box::pin(async move { inner.send(request).await })
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
