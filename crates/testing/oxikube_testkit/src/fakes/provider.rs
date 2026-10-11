//! Fakes of the metrics provider ports (E13-S05): [`FakeServiceProxyPort`] and
//! [`FakeProviderDetectorPort`].

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    DetectionResult, DetectionState, PromEndpoint, PromqlError, ProviderDetectorPort, ProxyRequest,
    ProxyResponse, ResourceReader, ServiceProxyPort, ServiceRef,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

// --- ServiceProxyPort --------------------------------------------------------------------

/// What a [`FakeServiceProxyPort`] answers with when nothing is queued: a function of the
/// service and the request. `None` falls through to the built-in fallback.
type Responder =
    dyn Fn(&ServiceRef, &ProxyRequest) -> Option<OxiResult<ProxyResponse>> + Send + Sync;

/// Queued responses of a [`FakeServiceProxyPort`].
#[derive(Debug, Default)]
pub struct ServiceProxyScripts {
    /// `send`.
    pub send: Script<ProxyResponse>,
}

/// One call made on a [`FakeServiceProxyPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceProxyCall {
    /// The service addressed.
    pub service: ServiceRef,
    /// The request path.
    pub path: String,
    /// The request parameters.
    pub params: Vec<(String, String)>,
}

/// Fake `ServiceProxyPort`. A queued response is used first, then the responder set with
/// [`respond_with`](Self::respond_with), then a fallback of `503` from the API server ("no
/// endpoints available"), the answer for a Service nothing serves.
#[derive(Default)]
pub struct FakeServiceProxyPort {
    script: ServiceProxyScripts,
    calls: CallLog<ServiceProxyCall>,
    responder: Mutex<Option<Arc<Responder>>>,
}

fake_plumbing!(FakeServiceProxyPort, ServiceProxyScripts, ServiceProxyCall);

impl std::fmt::Debug for FakeServiceProxyPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeServiceProxyPort")
            .field("script", &self.script)
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

impl FakeServiceProxyPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers every call that has no queued response by asking `responder`.
    pub fn respond_with(
        &self,
        responder: impl Fn(&ServiceRef, &ProxyRequest) -> Option<OxiResult<ProxyResponse>>
        + Send
        + Sync
        + 'static,
    ) {
        *self.responder.lock() = Some(Arc::new(responder));
    }

    /// A Prometheus API answer to `vector(1)` or any other query: `200` with an empty vector.
    pub fn prometheus_ok() -> ProxyResponse {
        Self::json(
            200,
            r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#,
        )
    }

    /// A response with `status` and a JSON `body`.
    pub fn json(status: u16, body: &str) -> ProxyResponse {
        ProxyResponse {
            status,
            content_type: Some("application/json".into()),
            body: body.as_bytes().to_vec(),
            truncated: false,
            api_server_message: None,
        }
    }

    /// The API server's own refusal (`status`) with its `message`.
    pub fn api_error(status: u16, message: &str) -> ProxyResponse {
        ProxyResponse {
            status,
            content_type: Some("application/json".into()),
            body: format!(r#"{{"kind":"Status","status":"Failure","message":"{message}"}}"#)
                .into_bytes(),
            truncated: false,
            api_server_message: Some(message.to_owned()),
        }
    }
}

#[async_trait]
impl ServiceProxyPort for FakeServiceProxyPort {
    async fn send(&self, service: &ServiceRef, request: ProxyRequest) -> OxiResult<ProxyResponse> {
        self.calls.record(ServiceProxyCall {
            service: service.clone(),
            path: request.path.clone(),
            params: request.params.clone(),
        });
        if let Some(queued) = self.script.send.pop() {
            return queued;
        }
        let responder = self.responder.lock().clone();
        if let Some(answer) = responder.and_then(|r| r(service, &request)) {
            return answer;
        }
        Ok(Self::api_error(
            503,
            &format!("no endpoints available for service \"{}\"", service.name),
        ))
    }
}

// --- ProviderDetectorPort ----------------------------------------------------------------

/// Queued results of a [`FakeProviderDetectorPort`].
#[derive(Debug, Default)]
pub struct ProviderDetectorScripts {
    /// `detect`.
    pub detect: Script<DetectionResult>,
    /// `check`: `Ok(())` for a reachable endpoint. Queue outcomes with
    /// [`push_check`](Self::push_check).
    pub check: Mutex<std::collections::VecDeque<Result<(), PromqlError>>>,
}

impl ProviderDetectorScripts {
    /// Queues the outcome of one `check`.
    pub fn push_check(&self, outcome: Result<(), PromqlError>) -> &Self {
        self.check.lock().push_back(outcome);
        self
    }
}

/// One call made on a [`FakeProviderDetectorPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderDetectorCall {
    /// `detect`.
    Detect,
    /// `check`, with the endpoint's location (credentials are never recorded).
    Check(String),
}

/// Fake `ProviderDetectorPort`. `detect` returns the next queued result, else `NotFound`;
/// `check` returns the next queued outcome, else reachable. An optional gate makes `detect`
/// wait, so a test can observe the `Detecting` state.
#[derive(Default)]
pub struct FakeProviderDetectorPort {
    script: ProviderDetectorScripts,
    calls: CallLog<ProviderDetectorCall>,
    gate: Mutex<Option<futures::channel::oneshot::Receiver<()>>>,
}

fake_plumbing!(
    FakeProviderDetectorPort,
    ProviderDetectorScripts,
    ProviderDetectorCall
);

impl std::fmt::Debug for FakeProviderDetectorPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeProviderDetectorPort")
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

impl FakeProviderDetectorPort {
    /// A fake whose detection finds nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the next `detect` wait until the returned sender fires (or is dropped).
    pub fn hold_next_detect(&self) -> futures::channel::oneshot::Sender<()> {
        let (tx, rx) = futures::channel::oneshot::channel();
        *self.gate.lock() = Some(rx);
        tx
    }
}

#[async_trait]
impl ProviderDetectorPort for FakeProviderDetectorPort {
    async fn detect(
        &self,
        _resources: Arc<dyn ResourceReader>,
        _proxy: Arc<dyn ServiceProxyPort>,
    ) -> DetectionResult {
        self.calls.record(ProviderDetectorCall::Detect);
        let gate = self.gate.lock().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        self.script
            .detect
            .pop()
            .and_then(Result::ok)
            .unwrap_or_else(|| DetectionResult::of(DetectionState::NotFound))
    }

    async fn check(
        &self,
        _proxy: Arc<dyn ServiceProxyPort>,
        endpoint: &PromEndpoint,
    ) -> Result<(), PromqlError> {
        self.calls
            .record(ProviderDetectorCall::Check(endpoint.location()));
        self.script.check.lock().pop_front().unwrap_or(Ok(()))
    }
}
