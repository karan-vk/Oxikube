//! A scripted [`LogSource`]: the log responses, pod states and pod watch a test wants, in order.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use futures::TryStreamExt;
use futures::stream::{self, BoxStream, StreamExt};
use jiff::{SignedDuration, Timestamp};
use oxikube_domain::log::LogLine;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{LogOptions, LogStream};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::logs::source::{
    ContainerInfo, ContainerState, LogSource, OpenRequest, PodEvent, PodInfo, PodPhase, Reader,
    RestartPolicy,
};
use crate::logs::{KubeLogs, LogsConfig};

/// The timestamp of line `n`: `2026-10-03T12:00:00Z` plus `n` x 100 ms.
pub(super) fn ts(n: i64) -> Timestamp {
    "2026-10-03T12:00:00Z"
        .parse::<Timestamp>()
        .unwrap()
        .checked_add(SignedDuration::from_millis(100 * n))
        .unwrap()
}

/// One wire line as the kubelet writes it.
pub(super) fn wire(n: i64, text: &str) -> Vec<u8> {
    format!("{} {text}\n", ts(n)).into_bytes()
}

/// What a scripted response does, in order.
#[derive(Clone)]
pub(super) enum Chunk {
    Bytes(Vec<u8>),
    /// The connection drops.
    Break,
    /// The server stops sending but keeps the connection open.
    Hang,
}

/// Lines `a..b` as `line {n}`.
pub(super) fn lines(range: std::ops::Range<i64>) -> Vec<Chunk> {
    range
        .map(|n| Chunk::Bytes(wire(n, &format!("line {n}"))))
        .collect()
}

/// A response with these chunks.
pub(super) struct Reply(pub(super) Vec<Chunk>);

enum Script {
    Reply(Reply),
    Raw(Reader),
    Fail(OxiError),
}

#[derive(Default)]
struct State {
    scripts: HashMap<String, VecDeque<Script>>,
    pods: HashMap<String, VecDeque<Option<PodInfo>>>,
    opens: Vec<(String, OpenRequest)>,
    watch: Option<mpsc::UnboundedReceiver<OxiResult<PodEvent>>>,
}

/// The scripted source; clones share state.
#[derive(Clone, Default)]
pub(super) struct FakeSource {
    state: Arc<Mutex<State>>,
}

impl FakeSource {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The next open of `pod`/`container` answers with `reply`.
    pub(super) fn reply(&self, pod: &str, container: &str, chunks: Vec<Chunk>) -> &Self {
        self.push(pod, container, Script::Reply(Reply(chunks)))
    }

    /// The next open of `pod`/`container` answers with this reader as is.
    pub(super) fn reply_raw(&self, pod: &str, container: &str, reader: Reader) -> &Self {
        self.push(pod, container, Script::Raw(reader))
    }

    /// The next open of `pod`/`container` fails with `error`.
    pub(super) fn fail(&self, pod: &str, container: &str, error: OxiError) -> &Self {
        self.push(pod, container, Script::Fail(error))
    }

    fn push(&self, pod: &str, container: &str, script: Script) -> &Self {
        self.state
            .lock()
            .scripts
            .entry(format!("{pod}/{container}"))
            .or_default()
            .push_back(script);
        self
    }

    /// `pod` reads as `info`; further calls move through the queue and the last one repeats.
    pub(super) fn pod_state(&self, info: PodInfo) -> &Self {
        self.state
            .lock()
            .pods
            .entry(info.name.clone())
            .or_default()
            .push_back(Some(info));
        self
    }

    /// `pod` does not exist (from the next queued state on).
    pub(super) fn pod_gone(&self, pod: &str) -> &Self {
        self.state
            .lock()
            .pods
            .entry(pod.to_owned())
            .or_default()
            .push_back(None);
        self
    }

    /// A pod watch the test feeds through the returned sender.
    pub(super) fn watch(&self) -> mpsc::UnboundedSender<OxiResult<PodEvent>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.state.lock().watch = Some(rx);
        tx
    }

    /// Every open so far as `(pod, request)`.
    pub(super) fn opens(&self) -> Vec<(String, OpenRequest)> {
        self.state.lock().opens.clone()
    }

    /// The opens of `pod`/`container`.
    pub(super) fn requests(&self, pod: &str) -> Vec<OpenRequest> {
        self.opens()
            .into_iter()
            .filter(|(p, _)| p == pod)
            .map(|(_, r)| r)
            .collect()
    }

    /// `KubeLogs` over this source with a config that keeps tests small.
    pub(super) fn logs(&self) -> KubeLogs {
        KubeLogs::with_source(Arc::new(self.clone()), LogsConfig::default())
    }

    pub(super) fn logs_with(&self, config: LogsConfig) -> KubeLogs {
        KubeLogs::with_source(Arc::new(self.clone()), config)
    }
}

#[async_trait]
impl LogSource for FakeSource {
    async fn open(&self, _namespace: &str, pod: &str, request: &OpenRequest) -> OxiResult<Reader> {
        let script = {
            let mut state = self.state.lock();
            state.opens.push((pod.to_owned(), request.clone()));
            state
                .scripts
                .get_mut(&format!("{pod}/{}", request.container))
                .and_then(VecDeque::pop_front)
        };
        match script {
            Some(Script::Reply(reply)) => Ok(reader(reply)),
            Some(Script::Raw(reader)) => Ok(reader),
            Some(Script::Fail(error)) => Err(error),
            None => Err(OxiError::not_found("no scripted response")),
        }
    }

    async fn pod(&self, _namespace: &str, name: &str) -> OxiResult<Option<PodInfo>> {
        let mut state = self.state.lock();
        match state.pods.get_mut(name) {
            Some(queue) if queue.len() > 1 => Ok(queue.pop_front().expect("queue not empty")),
            Some(queue) => Ok(queue.front().cloned().flatten()),
            None => Ok(Some(pod(name, &[("app", ContainerState::Running, 0)]))),
        }
    }

    fn watch_pods(&self, _: Option<&str>, _: &str) -> BoxStream<'static, OxiResult<PodEvent>> {
        let rx = self.state.lock().watch.take().expect("watch not scripted");
        stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) }).boxed()
    }
}

fn reader(reply: Reply) -> Reader {
    let parts = reply.0.into_iter().map(|chunk| match chunk {
        Chunk::Bytes(bytes) => stream::iter([Ok(bytes)]).boxed(),
        Chunk::Break => {
            stream::iter([Err(io::Error::from(io::ErrorKind::ConnectionReset))]).boxed()
        }
        Chunk::Hang => stream::pending().boxed(),
    });
    Box::pin(stream::iter(parts).flatten().into_async_read())
}

/// A pod `name` with `containers` as `(name, state, restart_count)`; restart policy `Always`,
/// phase `Running`.
pub(super) fn pod(name: &str, containers: &[(&str, ContainerState, i32)]) -> PodInfo {
    PodInfo {
        namespace: "ns".into(),
        name: name.into(),
        uid: format!("uid-{name}"),
        deleting: false,
        phase: PodPhase::Running,
        restart_policy: RestartPolicy::Always,
        default_container: None,
        containers: containers
            .iter()
            .map(|(name, state, restarts)| ContainerInfo {
                name: (*name).into(),
                restart_count: *restarts,
                state: *state,
                init: false,
            })
            .collect(),
    }
}

/// The text of every line, in order, until the stream ends or errors.
pub(super) async fn collect(stream: LogStream) -> Vec<OxiResult<LogLine>> {
    stream.collect().await
}

/// The texts of the lines in `items`, panicking on an error item.
pub(super) fn texts(items: &[OxiResult<LogLine>]) -> Vec<String> {
    items
        .iter()
        .map(|item| item.as_ref().expect("log line").text.clone())
        .collect()
}

/// Follow options for container `app`.
pub(super) fn follow() -> LogOptions {
    LogOptions::follow().container("app")
}

/// `n` expected line texts starting at `from`.
pub(super) fn expect_lines(range: std::ops::Range<i64>) -> Vec<String> {
    range.map(|n| format!("line {n}")).collect()
}
