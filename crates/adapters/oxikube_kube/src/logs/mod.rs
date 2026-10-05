//! `LogPort` on kube-rs: container logs as a `Stream<LogLine>` that survives reconnects (E04-S08).
//!
//! [`KubeLogs`] reads `pods/log` with `Api<Pod>::log_stream`. A followed stream is kept alive
//! through dropped connections, container restarts and API-server stream timeouts, without
//! duplicated lines and without gaps; see [`follow`] for the algorithm (ported from kdash's
//! `stream_container_logs`, MIT).
//!
//! | Piece | Where |
//! |---|---|
//! | `LogPort::stream_logs`: one container, `follow`/`since`/`tail`/`previous`/`timestamps` | `single` |
//! | reconnect loop: backoff, overlap, previous-instance gap fill, end conditions | `follow` |
//! | dedup of the replayed overlap by `(timestamp, text)` | `dedup` |
//! | bounded line reader, timestamp prefix | `line` |
//! | batching and the abort-on-drop channel stream | `stream` |
//! | [`KubeLogs::stream_containers`], [`KubeLogs::stream_selector`]: multi-container and label-selector fan-in | `fanin` |
//! | the kube-rs implementation of the source seam | `kube_source` |
//! | settings ([`LogsConfig`]) | `config` |
//!
//! # Lines
//!
//! Timestamps are always requested from the server and moved into [`LogLine::ts`]; the text
//! never carries the prefix, whether or not [`LogOptions::timestamps`] was set. That makes the
//! timestamp the dedup key, so lines an application repeats are kept. A line over
//! [`MAX_LOG_LINE_BYTES`](oxikube_domain::log::MAX_LOG_LINE_BYTES) is cut and flagged
//! `truncated`. Lines are not redacted here: they are the user's data and the viewer shows
//! them as written; redaction applies wherever they leave memory (non-negotiable 5), and the
//! adapter never writes line content to its own logs.
//!
//! # Throughput and memory
//!
//! Lines are read into one reusable buffer, cost one `String` each, and travel in batches of
//! [`LogsConfig::batch_size`] (flushed after [`LogsConfig::flush_interval`] when quiet) over a
//! bounded channel of [`LogsConfig::channel_batches`] batches. A consumer that stops reading
//! stops the reader and so the socket; nothing queues without bound.
//!
//! # Cancellation
//!
//! The returned stream owns its tasks. Dropping it aborts them and closes the connections.
//!
//! # Errors
//!
//! Failures go through [`classify`](crate::auth::classify). Opening the stream is the error of
//! [`stream_logs`](oxikube_ports::LogPort::stream_logs) itself (`NotFound`, `Forbidden`,
//! `Validation` for a container that has not started, ...). After that, a dropped connection
//! is retried; the stream ends with an `Err` item only for a failure retrying will not fix or
//! after [`LogsConfig::max_open_failures`] failed reopens in a row. It ends without error when
//! the pod is deleted or the container has finished and will not restart.

mod config;
mod dedup;
mod fanin;
mod follow;
mod kube_source;
mod line;
mod single;
mod source;
mod stream;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use kube::Client;
use oxikube_domain::OxiResult;
use oxikube_ports::{LogOptions, LogPort, LogStream};

pub use config::LogsConfig;
pub use fanin::ContainerSelection;
use kube_source::KubeSource;
use source::LogSource;

/// Container logs for one cluster. Cheap to clone; clones share the client.
#[derive(Clone)]
pub struct KubeLogs {
    source: Arc<dyn LogSource>,
    config: LogsConfig,
}

impl KubeLogs {
    /// Reads logs through `client`.
    pub fn new(client: Client) -> Self {
        Self::with_config(client, LogsConfig::default())
    }

    /// As [`new`](Self::new) with explicit settings.
    pub fn with_config(client: Client, config: LogsConfig) -> Self {
        Self::with_source(Arc::new(KubeSource::new(client)), config)
    }

    pub(crate) fn with_source(source: Arc<dyn LogSource>, config: LogsConfig) -> Self {
        Self { source, config }
    }

    /// The settings in effect.
    pub fn config(&self) -> &LogsConfig {
        &self.config
    }

    /// Streams the logs of several containers of one pod as one stream; each line's
    /// `container` says where it came from. `options.container` is ignored; `tail_lines` and
    /// `since` apply to each container separately.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown pod, `Validation` for an unknown container name. A container
    /// that cannot be read later yields an `Err` item without ending the others.
    pub async fn stream_containers(
        &self,
        namespace: &str,
        pod: &str,
        containers: &ContainerSelection,
        options: &LogOptions,
    ) -> OxiResult<LogStream> {
        fanin::stream_containers(
            &self.source,
            &self.config,
            namespace,
            pod,
            containers,
            options,
        )
        .await
    }

    /// Streams the logs of every container of every pod matching the label `selector`
    /// (`namespace` `None` is all namespaces). Each line's `pod` and `container` say where it
    /// came from. With `follow`, pods that start matching later join the stream and it runs
    /// until dropped; without it, the pods present now are read to their end.
    ///
    /// # Errors
    ///
    /// `Validation` for an empty selector. Failing to list or watch pods (`Forbidden`, ...) is
    /// the stream's first and only item.
    pub fn stream_selector(
        &self,
        namespace: Option<&str>,
        selector: &str,
        options: &LogOptions,
    ) -> OxiResult<LogStream> {
        fanin::stream_selector(&self.source, &self.config, namespace, selector, options)
    }
}

#[async_trait]
impl LogPort for KubeLogs {
    async fn stream_logs(
        &self,
        namespace: &str,
        pod: &str,
        options: &LogOptions,
    ) -> OxiResult<LogStream> {
        single::stream_one(&self.source, &self.config, namespace, pod, options).await
    }
}

impl std::fmt::Debug for KubeLogs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeLogs")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
