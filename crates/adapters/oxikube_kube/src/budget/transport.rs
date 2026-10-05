//! The byte counter of a feed: a client that counts every response body chunk it receives.
//!
//! The budget gives each feed its own [`KubeResources`] whose client is the session client
//! behind [`CountBytes`], so lists, watches and Table requests of that feed are counted and
//! nothing else is. The count is of decoded bytes (kube's client decompresses below this
//! layer), which is what the process actually parses. Connection pool, auth and retries stay
//! the session client's.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::{Request, Response};
use http_body::{Body as HttpBody, Frame, SizeHint};
use kube::Client;
use kube::client::Body;
use tower::Service;

use super::counters::ByteCounter;
use crate::resources::KubeResources;

impl KubeResources {
    /// A clone whose requests count their response bytes into `bytes`.
    pub(super) fn counting_bytes(&self, bytes: ByteCounter) -> KubeResources {
        let mut counted = self.clone();
        counted.client = counting_client(&self.client, bytes);
        counted
    }
}

/// `client` with every response body counted into `bytes`.
pub(super) fn counting_client(client: &Client, bytes: ByteCounter) -> Client {
    let service = CountBytes {
        client: client.clone(),
        bytes,
    };
    Client::new(service, client.default_namespace())
}

/// The service behind [`counting_client`].
#[derive(Clone)]
struct CountBytes {
    client: Client,
    bytes: ByteCounter,
}

impl Service<Request<Body>> for CountBytes {
    type Response = Response<CountedBody>;
    type Error = kube::Error;
    type Future = BoxFuture<'static, Result<Self::Response, kube::Error>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let client = self.client.clone();
        let bytes = self.bytes.clone();
        Box::pin(async move {
            let response = client.send(request).await?;
            Ok(response.map(|inner| CountedBody { inner, bytes }))
        })
    }
}

/// A response body that adds the length of every data frame to its counter.
struct CountedBody {
    inner: Body,
    bytes: ByteCounter,
}

impl HttpBody for CountedBody {
    type Data = <Body as HttpBody>::Data;
    type Error = <Body as HttpBody>::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(Some(Ok(frame))) = &polled {
            if let Some(data) = frame.data_ref() {
                self.bytes.add(data.len());
            }
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
