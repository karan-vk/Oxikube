//! Fan-in: every container of a pod, or every pod matching a label selector, as one stream.
//!
//! Each container gets its own [`Follower`] (so each reconnects and dedups on its own) and
//! its own batching [`Sink`] into one shared bounded channel; the consumer sees the
//! interleaving in arrival order. The source of a line is its `pod` and `container` fields.
//!
//! Unlike a single stream, a fan-in stream does not end when one source fails: a container
//! that cannot be read yields an `Err` item and the others carry on. The stream ends when
//! every source has ended (for a selector with `follow`, when it is dropped).
//!
//! # Selector fan-in
//!
//! A watch on the pods matching the selector drives it. Containers that have started when
//! the stream opens use the caller's options (`tail_lines`, `since`); pods that appear later
//! are read from their first line, since they are new. A container is followed from the
//! moment it has started (a pending pod joins when its container comes up), once per pod
//! uid, so a pod recreated under the same name is followed again. At most
//! [`LogsConfig::max_fanin_streams`] containers are followed at once.

use std::collections::HashSet;
use std::sync::Arc;

use futures::StreamExt;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{LogOptions, LogStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::config::LogsConfig;
use super::follow::{Follower, Target};
use super::source::{LogSource, PodEvent, PodInfo};
use super::stream::{ChannelStream, Sink};

/// Which containers of a pod [`KubeLogs::stream_containers`](super::KubeLogs::stream_containers) reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerSelection {
    /// Every container, init and ephemeral ones included (`kubectl logs --all-containers`).
    /// Without `follow`, containers that never started are skipped; with it they are
    /// followed once they start.
    All,
    /// These containers; an unknown name is a `Validation` error.
    Named(Vec<String>),
}

/// All (or the named) containers of one pod.
pub(super) async fn stream_containers(
    source: &Arc<dyn LogSource>,
    config: &LogsConfig,
    namespace: &str,
    pod: &str,
    selection: &ContainerSelection,
    options: &LogOptions,
) -> OxiResult<LogStream> {
    let info = source
        .pod(namespace, pod)
        .await?
        .ok_or_else(|| OxiError::not_found(format!("pod {namespace}/{pod} does not exist")))?;
    let mut names: Vec<String> = match selection {
        ContainerSelection::All => info
            .containers
            .iter()
            .filter(|c| options.follow || c.has_started())
            .map(|c| c.name.clone())
            .collect(),
        ContainerSelection::Named(named) => {
            if let Some(unknown) = named.iter().find(|n| info.container(n).is_none()) {
                return Err(OxiError::validation(format!(
                    "pod {pod} has no container named `{unknown}`"
                )));
            }
            named.clone()
        }
    };
    names.truncate(config.max_fanin_streams);
    let (tx, rx) = mpsc::channel(config.channel_batches.max(1));
    let mut tasks = JoinSet::new();
    for name in names {
        let follower = follower(
            source,
            config,
            &info,
            &name,
            options.clone(),
            Sink::new(tx.clone(), config),
        );
        tasks.spawn(follower.run(None));
    }
    Ok(Box::pin(ChannelStream::new(rx, tasks)))
}

/// Every pod matching `selector`, joined as they appear.
pub(super) fn stream_selector(
    source: &Arc<dyn LogSource>,
    config: &LogsConfig,
    namespace: Option<&str>,
    selector: &str,
    options: &LogOptions,
) -> OxiResult<LogStream> {
    if selector.trim().is_empty() {
        return Err(OxiError::validation("a label selector is required"));
    }
    let (tx, rx) = mpsc::channel(config.channel_batches.max(1));
    let watch = source.watch_pods(namespace, selector);
    let manager = Manager {
        source: Arc::clone(source),
        config: config.clone(),
        options: options.clone(),
        sink: Sink::new(tx, config),
        followers: JoinSet::new(),
        started: HashSet::new(),
        warned_cap: false,
    };
    let mut tasks = JoinSet::new();
    tasks.spawn(manager.run(watch));
    Ok(Box::pin(ChannelStream::new(rx, tasks)))
}

fn follower(
    source: &Arc<dyn LogSource>,
    config: &LogsConfig,
    pod: &PodInfo,
    container: &str,
    mut options: LogOptions,
    sink: Sink,
) -> Follower {
    options.container = Some(container.to_owned());
    Follower::new(
        Arc::clone(source),
        config.clone(),
        Target {
            namespace: pod.namespace.clone(),
            pod: Arc::from(pod.name.as_str()),
            container: Arc::from(container),
        },
        options,
        sink,
        Some(pod),
    )
}

/// Watches the pod set and starts a follower per newly started container.
struct Manager {
    source: Arc<dyn LogSource>,
    config: LogsConfig,
    options: LogOptions,
    sink: Sink,
    /// Owned here, so aborting the manager task aborts every follower.
    followers: JoinSet<()>,
    /// `(pod uid, container)` pairs already followed.
    started: HashSet<(String, String)>,
    warned_cap: bool,
}

impl Manager {
    async fn run(mut self, mut watch: futures::stream::BoxStream<'static, OxiResult<PodEvent>>) {
        loop {
            tokio::select! {
                event = watch.next() => match event {
                    None => break,
                    Some(Err(err)) => {
                        let _ = self.sink.fail(err).await;
                        return;
                    }
                    // Without `follow` the pods present now are all there is to read.
                    Some(Ok(PodEvent::InitDone)) if !self.options.follow => break,
                    Some(Ok(PodEvent::InitDone)) => {}
                    Some(Ok(PodEvent::Pod { pod, initial })) => {
                        if initial || self.options.follow {
                            self.join(&pod, initial);
                        }
                    }
                },
                // Reap finished followers so the cap counts live ones.
                Some(_) = self.followers.join_next(), if !self.followers.is_empty() => {}
            }
        }
        drop(watch);
        // Dropping `self.sink` is not enough to end the stream: the followers hold senders
        // too, so wait for them (they run until their containers finish).
        while self.followers.join_next().await.is_some() {}
    }

    /// Starts followers for the containers of `pod` that have started and are not followed yet.
    fn join(&mut self, pod: &PodInfo, initial: bool) {
        for container in pod.containers.iter().filter(|c| c.has_started()) {
            let key = (pod.uid.clone(), container.name.clone());
            if self.started.contains(&key) {
                continue;
            }
            if self.followers.len() >= self.config.max_fanin_streams {
                if !std::mem::replace(&mut self.warned_cap, true) {
                    tracing::warn!(
                        limit = self.config.max_fanin_streams,
                        "log fan-in is at its container limit; further containers are not followed"
                    );
                }
                return;
            }
            self.started.insert(key);
            let mut options = self.options.clone();
            if !initial {
                // A pod that appeared after the stream opened is new: read it from the start.
                options.tail_lines = None;
                options.since = None;
            }
            let follower = follower(
                &self.source,
                &self.config,
                pod,
                &container.name,
                options,
                self.sink.sibling(),
            );
            self.followers.spawn(follower.run(None));
        }
    }
}
