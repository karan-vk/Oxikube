//! One container's stream: the [`LogPort`](oxikube_ports::LogPort) implementation's core.

use std::sync::Arc;

use futures::future;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{LogOptions, LogStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::config::LogsConfig;
use super::follow::{Follower, Target, first_request};
use super::source::{LogSource, PodInfo};
use super::stream::{ChannelStream, Sink};

/// Opens the log of one container and starts the task that keeps it flowing.
///
/// The first open happens here, so a missing pod, a denied `pods/log` or a container that has
/// not started is the error of this call. Errors after the first line arrive as the stream's
/// last item.
pub(super) async fn stream_one(
    source: &Arc<dyn LogSource>,
    config: &LogsConfig,
    namespace: &str,
    pod: &str,
    options: &LogOptions,
) -> OxiResult<LogStream> {
    let (container, info, reader) = match options.container.as_deref() {
        Some(name) => {
            let request = first_request(name, options);
            // The pod read seeds the restart and identity checks; it must not slow the open.
            let (reader, info) = future::join(
                source.open(namespace, pod, &request),
                source.pod(namespace, pod),
            )
            .await;
            let info = info.ok().flatten();
            let reader = match reader {
                Ok(reader) => reader,
                // The server answers 400 for a container the pod does not have; the port
                // contract is `NotFound`.
                Err(err) if info.as_ref().is_some_and(|i| i.container(name).is_none()) => {
                    return Err(OxiError::not_found(format!(
                        "pod {namespace}/{pod} has no container named `{name}`"
                    ))
                    .with_source(err));
                }
                Err(err) => return Err(err),
            };
            (name.to_owned(), info, reader)
        }
        None => {
            let info = source.pod(namespace, pod).await?.ok_or_else(|| {
                OxiError::not_found(format!("pod {namespace}/{pod} does not exist"))
            })?;
            let name = default_container(&info)?;
            let reader = source
                .open(namespace, pod, &first_request(&name, options))
                .await?;
            (name, Some(info), reader)
        }
    };
    let (tx, rx) = mpsc::channel(config.channel_batches.max(1));
    let follower = Follower::new(
        Arc::clone(source),
        config.clone(),
        Target {
            namespace: namespace.to_owned(),
            pod: Arc::from(pod),
            container: Arc::from(container),
        },
        options.clone(),
        Sink::new(tx, config),
        info.as_ref(),
    );
    let mut tasks = JoinSet::new();
    tasks.spawn(follower.run(Some(reader)));
    Ok(Box::pin(ChannelStream::new(rx, tasks)))
}

/// The container a request without a name reads, or why there is none.
fn default_container(pod: &PodInfo) -> OxiResult<String> {
    pod.default_container_name()
        .map(str::to_owned)
        .ok_or_else(|| {
            let names: Vec<_> = pod
                .containers
                .iter()
                .filter(|c| !c.init)
                .map(|c| c.name.as_str())
                .collect();
            OxiError::validation(format!(
                "a container name must be specified for pod {}; choose one of: {}",
                pod.name,
                names.join(", ")
            ))
        })
}
