//! Debug containers: an ephemeral container added to a running pod, then attached to
//! (`kubectl debug`).

use std::time::Duration;

use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecOptions, ExecPort, ExecSession};

use super::pods::Pods;
use super::wait::Container;
use crate::subresource::{EphemeralContainerSpec, ephemeral_container_patch, segment};

/// How long to wait for a debug container to start when the caller has no opinion: long
/// enough for an image pull.
pub const DEFAULT_DEBUG_START_TIMEOUT: Duration = Duration::from_secs(60);

/// Adds `spec` to `namespace/pod`, waits for it to run and attaches to it.
pub(super) async fn attach_debug(
    exec: &dyn ExecPort,
    pods: &dyn Pods,
    namespace: &str,
    pod: &str,
    spec: &EphemeralContainerSpec,
    start_timeout: Duration,
) -> OxiResult<ExecSession> {
    segment("a namespace", namespace)?;
    segment("a pod name", pod)?;
    segment("a container name", &spec.name)?;
    if spec.image.trim().is_empty() {
        return Err(OxiError::validation("the debug container image is blank"));
    }
    let patch = ephemeral_container_patch(spec);
    pods.add_ephemeral_container(namespace, pod, &patch.body)
        .await?;
    tracing::info!(namespace, pod, container = %spec.name, "debug container added");
    pods.wait_running(
        namespace,
        pod,
        &Container::Ephemeral(spec.name.clone()),
        start_timeout,
    )
    .await?;
    let options = ExecOptions {
        container: Some(spec.name.clone()),
        stdin: spec.stdin,
        stdout: true,
        stderr: !spec.tty,
        tty: spec.tty,
    };
    exec.attach(namespace, pod, &options).await
}
