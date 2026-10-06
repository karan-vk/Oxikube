//! [`WarningLayer`]: a tower layer that publishes the `Warning` headers of every response.

use std::task::{Context, Poll};

use futures::future::BoxFuture;
use http::header::WARNING;
use http::{Request, Response};
use tower::{Layer, Service};

use super::header::parse;
use super::hub::WarningSink;

/// Wraps a client's HTTP service so each response's `Warning` headers reach `sink`.
///
/// The headers are read when the response head arrives, before the body (a watch's body never
/// ends, its warnings come with the head).
#[derive(Clone, Debug)]
pub struct WarningLayer {
    sink: WarningSink,
}

impl WarningLayer {
    /// A layer publishing to `sink`.
    pub fn new(sink: WarningSink) -> Self {
        Self { sink }
    }
}

impl<S> Layer<S> for WarningLayer {
    type Service = WarningService<S>;

    fn layer(&self, inner: S) -> WarningService<S> {
        WarningService {
            inner,
            sink: self.sink.clone(),
        }
    }
}

/// The service [`WarningLayer`] builds.
#[derive(Clone, Debug)]
pub struct WarningService<S> {
    inner: S,
    sink: WarningSink,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for WarningService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: 'static,
    ResBody: 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        let future = self.inner.call(request);
        let sink = self.sink.clone();
        Box::pin(async move {
            let response = future.await?;
            for value in response.headers().get_all(WARNING) {
                if let Ok(value) = value.to_str() {
                    for warning in parse(value) {
                        sink.publish(warning);
                    }
                }
            }
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;
    use oxikube_domain::ids::ContextName;
    use oxikube_ports::{ApiWarning, WarningPort as _};
    use tower::ServiceExt as _;

    use super::*;
    use crate::warnings::WarningHub;

    #[tokio::test]
    async fn every_warning_header_of_a_response_is_published_in_order() {
        let hub = WarningHub::default();
        let context = ContextName::new("ctx");
        let mut warnings = hub.port(&context).subscribe();
        let inner = tower::service_fn(|_: Request<()>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .header(WARNING, r#"299 - "first""#)
                    .header(WARNING, r#"299 - "second""#)
                    .body("body")
                    .unwrap(),
            )
        });
        let service = WarningLayer::new(hub.sink(&context)).layer(inner);
        let response = service.oneshot(Request::new(())).await.unwrap();
        assert_eq!(*response.body(), "body", "the response passes through");
        assert_eq!(warnings.next().await, Some(ApiWarning::new("first")));
        assert_eq!(warnings.next().await, Some(ApiWarning::new("second")));
    }

    #[tokio::test]
    async fn a_response_without_warnings_publishes_nothing() {
        let hub = WarningHub::default();
        let context = ContextName::new("ctx");
        let mut warnings = hub.port(&context).subscribe();
        let inner = tower::service_fn(|_: Request<()>| async {
            Ok::<_, std::convert::Infallible>(Response::new(()))
        });
        let service = WarningLayer::new(hub.sink(&context)).layer(inner);
        service.oneshot(Request::new(())).await.unwrap();
        let next =
            tokio::time::timeout(std::time::Duration::from_millis(20), warnings.next()).await;
        assert!(next.is_err(), "nothing was published: {next:?}");
    }
}
