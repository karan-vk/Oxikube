//! The debug-container half of [`ExecService`]: defaults for the dialog, planning, and opening.

use std::sync::Arc;

use oxikube_domain::command::{DEFAULT_DEBUG_COMMAND, DEFAULT_DEBUG_IMAGE};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::ContainerKind;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::DebugContainerSpec;

use super::request::{DebugPlan, DebugRequest, plan_debug};
use crate::exec::containers::{ExecContainer, PodContainers};
use crate::exec::notice::{NoticeBackend, notice_line};
use crate::exec::service::{ExecService, read_pod};
use crate::guard::Mutation;

/// What the debug dialog starts with for a pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugDefaults {
    /// The pod.
    pub pod: ResourceRef,
    /// The image: the one last used in this cluster, else [`DEFAULT_DEBUG_IMAGE`].
    pub image: String,
    /// The program: [`DEFAULT_DEBUG_COMMAND`].
    pub command: String,
    /// The containers the debug container can share processes with, in the pod's order.
    pub targets: Vec<ExecContainer>,
    /// The index in `targets` of the pod's default container (the one `kubectl` opens).
    pub target: usize,
}

/// What [`ExecService::open_debug`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugOpened {
    /// What was planned: the container's name, image and target.
    pub plan: DebugPlan,
    /// The dispatch was a dry run: the request was checked against the pod and nothing changed.
    pub dry_run: bool,
}

impl ExecService {
    /// What the debug dialog starts with for `pod`. Reads the pod.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown pod, `Validation` for a pod with no container to share processes
    /// with, `Conflict` for a cluster that is not connected, and the read's own error.
    pub async fn debug_defaults(&self, pod: &ResourceRef) -> OxiResult<DebugDefaults> {
        let (_, reader) = self.ports(&pod.cluster)?;
        let resource = read_pod(&reader, pod).await?;
        let containers = PodContainers::of(&resource);
        let targets: Vec<ExecContainer> = containers
            .containers()
            .iter()
            .filter(|c| c.kind != ContainerKind::Ephemeral)
            .cloned()
            .collect();
        let default = containers
            .default_container()
            .and_then(|d| targets.iter().position(|c| c.name == d.name))
            .unwrap_or(0);
        if targets.is_empty() {
            return Err(OxiError::validation(
                "the pod has no container to share processes with",
            ));
        }
        Ok(DebugDefaults {
            pod: pod.clone(),
            image: self
                .debug
                .last_image(&pod.cluster)
                .map_or_else(|| DEFAULT_DEBUG_IMAGE.to_owned(), |image| image.to_string()),
            command: DEFAULT_DEBUG_COMMAND.to_owned(),
            targets,
            target: default,
        })
    }

    /// What `request` would add to its pod: every default filled in, the target and the name
    /// checked against the pod as the cluster returns it now. Reads the pod, changes nothing.
    ///
    /// # Errors
    ///
    /// As [`plan_debug`], plus `NotFound` for an unknown pod and `Conflict` for a cluster that is
    /// not connected.
    pub async fn plan_debug(&self, request: &DebugRequest) -> OxiResult<DebugPlan> {
        request.check_fields()?;
        let (_, reader) = self.ports(&request.pod.cluster)?;
        let resource = read_pod(&reader, &request.pod).await?;
        plan_debug(request, &resource)
    }

    /// Adds the debug container to its pod, waits for it to run and attaches to it. Runs inside
    /// the guarded `pod::Debug` command: `mutation` is the guard's permit for this cluster.
    ///
    /// The attached session is kept until the terminal opened for `plan.name` claims it with
    /// [`attach`](Self::attach); a dry-run permit stops after planning.
    ///
    /// # Errors
    ///
    /// Everything [`plan_debug`](Self::plan_debug) can say; `Forbidden` for a cluster that went
    /// read-only meanwhile or an account without `patch` on `pods/ephemeralcontainers`;
    /// `Unsupported` for a server without ephemeral containers; `Timeout` when the container did
    /// not run within the request's timeout; and the API server's own message when it refuses the
    /// patch (an admission policy, a finished pod).
    pub async fn open_debug(
        &self,
        mutation: &Mutation,
        request: &DebugRequest,
    ) -> OxiResult<DebugOpened> {
        let cluster = &request.pod.cluster;
        if mutation.cluster() != cluster {
            return Err(OxiError::internal(
                "the guard's permit is for another cluster than the pod's",
            ));
        }
        let plan = self.plan_debug(request).await?;
        if mutation.dry_run() {
            return Ok(DebugOpened {
                plan,
                dry_run: true,
            });
        }
        let (port, _) = self.ports(cluster)?;
        // The guard checked read-only at admission; the patch may come long after (a pod read,
        // a confirmation): look once more, as the guard's writer does for a delete.
        if self.sessions.get(cluster).is_none_or(|s| s.read_only()) {
            return Err(OxiError::forbidden(
                "the cluster is read-only: a debug container cannot be added",
            ));
        }
        let mut spec = DebugContainerSpec::new(request.pod.clone(), plan.image.clone());
        spec.name = Some(plan.name.clone());
        spec.target_container = Some(plan.target.to_string());
        spec.command.clone_from(&plan.command);
        spec.start_timeout = request.start_timeout;
        let backend = port.create_debug_container(&spec).await?;
        tracing::info!(pod = %request.pod.name, container = %plan.name, "debug container running");
        self.debug.remember_image(cluster, &plan.image);
        let notice = notice_line(&plan.notice(&request.pod));
        self.debug.hold(
            &request.pod,
            &plan.name,
            Box::new(NoticeBackend::new(backend, notice)),
        );
        Ok(DebugOpened {
            plan,
            dry_run: false,
        })
    }

    /// Drops the session [`open_debug`](Self::open_debug) kept for `container` of `pod`, ending
    /// it: for a terminal that could not be opened. The container stays in the pod.
    pub fn discard_debug(&self, pod: &ResourceRef, container: &str) {
        drop(self.debug.claim(pod, container));
    }

    /// The image last used for a debug container in `cluster`'s pods this session.
    pub fn last_debug_image(&self, cluster: &oxikube_domain::ids::ClusterId) -> Option<Arc<str>> {
        self.debug.last_image(cluster)
    }

    /// How many opened debug sessions wait for their terminal.
    pub fn unclaimed_debug_sessions(&self) -> usize {
        self.debug.unclaimed()
    }
}
