//! When a container is ready to be exec'd into: reading it off the pod's status.

use k8s_openapi::api::core::v1::{ContainerStatus, Pod};

use crate::auth::redacted_line;

/// The container to wait for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Container {
    /// A regular container: `status.containerStatuses`.
    Regular(String),
    /// An ephemeral (debug) container: `status.ephemeralContainerStatuses`.
    Ephemeral(String),
}

impl Container {
    pub(super) fn name(&self) -> &str {
        match self {
            Self::Regular(name) | Self::Ephemeral(name) => name,
        }
    }
}

/// Where a container is on its way to running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Readiness {
    /// Not running yet; waiting can still help (scheduling, pulling, creating).
    Waiting,
    /// Running: exec can attach.
    Running,
    /// It cannot become running; the reason is one redacted line.
    Failed(String),
}

/// Waiting reasons that do not resolve by waiting longer.
const FATAL_WAITING: &[&str] = &[
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "ErrImageNeverPull",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
    "CrashLoopBackOff",
];

/// The readiness of `container` in `pod`.
pub(super) fn readiness(pod: &Pod, container: &Container) -> Readiness {
    if pod.metadata.deletion_timestamp.is_some() {
        return Readiness::Failed("the pod is terminating".into());
    }
    let status = pod.status.as_ref();
    let statuses: &[ContainerStatus] = match container {
        Container::Regular(_) => status.and_then(|s| s.container_statuses.as_deref()),
        Container::Ephemeral(_) => status.and_then(|s| s.ephemeral_container_statuses.as_deref()),
    }
    .unwrap_or_default();
    let state = statuses
        .iter()
        .find(|s| s.name == container.name())
        .and_then(|s| s.state.as_ref());
    if let Some(state) = state {
        if state.running.is_some() {
            return Readiness::Running;
        }
        if let Some(done) = &state.terminated {
            return Readiness::Failed(format!("the container exited with code {}", done.exit_code));
        }
        if let Some(waiting) = &state.waiting {
            let reason = waiting.reason.as_deref().unwrap_or_default();
            if FATAL_WAITING.contains(&reason) {
                let message = waiting.message.as_deref().map(redacted_line);
                return Readiness::Failed(match message {
                    Some(message) if !message.is_empty() => format!("{reason}: {message}"),
                    _ => reason.to_owned(),
                });
            }
        }
    }
    match status.and_then(|s| s.phase.as_deref()) {
        Some(phase @ ("Failed" | "Succeeded")) => {
            let why = status
                .and_then(|s| s.reason.as_deref().zip(s.message.as_deref()))
                .map(|(reason, message)| format!(": {reason}: {}", redacted_line(message)))
                .or_else(|| status?.reason.as_ref().map(|reason| format!(": {reason}")));
            Readiness::Failed(format!(
                "the pod is {}{}",
                phase.to_lowercase(),
                why.unwrap_or_default()
            ))
        }
        _ => Readiness::Waiting,
    }
}
