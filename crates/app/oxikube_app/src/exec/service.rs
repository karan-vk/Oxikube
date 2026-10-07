//! [`ExecService`]: opens shells, attaches and commands in pod containers over the session's
//! [`ExecPort`](oxikube_ports::ExecPort).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};
use oxikube_ports::{AttachTarget, ExecPort, ExecTarget, ResourceReader, TerminalBackend};
use parking_lot::Mutex;

use super::containers::{ContainerPlan, PodContainers, container_to_open, plan_container};
use super::debug::DebugState;
use super::failure::{explain_open, no_shell};
use super::node_shell::{OpenShells, Permits};
use super::notice::{NoticeBackend, notice_line};
use super::shell::{Probe, probe_shell};
use crate::audit::AuditLog;
use crate::session::ClusterSessionManager;

/// How many pods' last container choice is kept before the oldest are forgotten.
const MAX_REMEMBERED_PODS: usize = 512;

/// How `pod::Shell` finds a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOptions {
    /// The shells to try, first to last (`terminal.exec_shells`). Empty uses `bash`, then `sh`.
    pub shells: Vec<String>,
    /// How long one probe may take before the open fails with `Timeout`.
    pub probe_timeout: Duration,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            shells: Vec::new(),
            probe_timeout: Duration::from_secs(15),
        }
    }
}

impl ShellOptions {
    /// Options with the chain `shells`.
    pub fn with_shells(shells: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            shells: shells.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    fn chain(&self) -> Vec<String> {
        if self.shells.is_empty() {
            vec!["bash".to_owned(), "sh".to_owned()]
        } else {
            self.shells.clone()
        }
    }
}

/// Opens interactive sessions in pod containers: a shell with the `bash`-then-`sh` fallback,
/// an attach, or one command. See the [module docs](super).
///
/// Plain async Rust over the ports: it reads the pod through the session's `ResourceReader`
/// and opens sessions through its `ExecPort`, so tests use the fakes. It does not apply the
/// read-only policy or audit: those are the [`MutationGuard`](crate::MutationGuard)'s, on the
/// `pod::Shell`, `pod::Attach` and `pod::Exec` commands that lead here. It never logs or keeps
/// what a session sends or receives.
pub struct ExecService {
    pub(super) sessions: ClusterSessionManager,
    /// The container opened last in each pod this session, to preselect in the picker.
    last: Mutex<HashMap<ResourceRef, Arc<str>>>,
    /// Debug containers (E09-S10): the last image used per cluster, and the sessions opened and
    /// not yet claimed by their terminal.
    pub(super) debug: DebugState,
    /// The guarded `node::Shell` commands waiting for their terminal (E09-S09).
    pub(super) permits: Permits,
    /// The (cluster, namespace) pairs whose leftover node shell pods were swept this run.
    pub(super) swept: Mutex<HashSet<(ClusterId, String)>>,
    /// The node shells open now, for the app's quit.
    pub(super) open_shells: OpenShells,
    /// Where the end of a node shell is audited; `None` writes no second record (tests).
    pub(super) audit: OnceLock<Arc<AuditLog>>,
}

impl std::fmt::Debug for ExecService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecService").finish_non_exhaustive()
    }
}

impl ExecService {
    /// A service over the app's sessions.
    pub fn new(sessions: ClusterSessionManager) -> Self {
        Self {
            sessions,
            last: Mutex::new(HashMap::new()),
            debug: DebugState::default(),
            permits: Permits::default(),
            swept: Mutex::new(HashSet::new()),
            open_shells: OpenShells::default(),
            audit: OnceLock::new(),
        }
    }

    /// Makes the service write the audit record of a node shell's end (the pod's deletion) to
    /// `audit`, the [`MutationGuard`](crate::MutationGuard)'s log
    /// ([`audit_handle`](crate::MutationGuard::audit_handle)). Set once, after the bus is built
    /// (the bus needs this service for its `node::Shell` handler, so the log comes second):
    /// `false` when one was set already.
    pub fn set_audit(&self, audit: Arc<AuditLog>) -> bool {
        self.audit.set(audit).is_ok()
    }

    /// Which container opening a session in `pod` uses: the one named (`requested`), the only
    /// one, or the choices for a picker with the last choice for this pod (else the default)
    /// preselected. Reads the pod.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown pod or container, `Validation` for a pod with nothing to open,
    /// `Conflict` for a cluster that is not connected, and the read's own error (`Forbidden`...).
    pub async fn plan(
        &self,
        pod: &ResourceRef,
        requested: Option<&str>,
    ) -> OxiResult<ContainerPlan> {
        let (_, reader) = self.ports(&pod.cluster)?;
        let resource = read_pod(&reader, pod).await?;
        let remembered = self.last_container(pod);
        plan_container(
            &PodContainers::of(&resource),
            requested,
            remembered.as_deref(),
        )
    }

    /// The container opened last in `pod` in this session.
    pub fn last_container(&self, pod: &ResourceRef) -> Option<Arc<str>> {
        self.last.lock().get(pod).cloned()
    }

    /// Records `container` as the one opened last in `pod` (the picker preselects it next time).
    pub fn remember(&self, pod: &ResourceRef, container: &str) {
        let mut last = self.last.lock();
        if last.len() >= MAX_REMEMBERED_PODS && !last.contains_key(pod) {
            // A long session through many pods: forgetting the old choices costs one question.
            last.clear();
        }
        last.insert(pod.clone(), Arc::from(container));
    }

    /// Opens an interactive shell in `container` of `pod` (`None`: the pod's default container).
    ///
    /// Tries each shell of `options` with a quick exec and opens the first the container has.
    /// The terminal starts with one line saying which shell opened, and which were missing.
    ///
    /// # Errors
    ///
    /// `Unsupported` when no shell of the chain exists (a distroless image: the message points
    /// to a debug container), `Forbidden` without `create` on `pods/exec`, `NotFound`,
    /// `Conflict` for a container that is not running or a terminating pod (retryable),
    /// `Network` / `Timeout` for the connection. Messages name the pod and never carry output.
    pub async fn open_shell(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        options: &ShellOptions,
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        let (port, reader) = self.ports(&pod.cluster)?;
        let container = self.resolve(&reader, pod, container).await?;
        let mut missing = Vec::new();
        for shell in options.chain() {
            let found = probe_shell(
                &*port,
                pod,
                container.as_deref(),
                &shell,
                options.probe_timeout,
            )
            .await
            .map_err(explain_open)?;
            if found == Probe::Missing {
                missing.push(shell);
                continue;
            }
            let mut target = ExecTarget::interactive(pod.clone(), vec![shell.clone()]);
            target.container = container.as_deref().map(str::to_owned);
            let backend = port.exec(&target).await.map_err(explain_open)?;
            let place = place(pod, container.as_deref());
            let text = if missing.is_empty() {
                format!("{shell} in {place}")
            } else {
                format!("{} not found, using {shell} in {place}", missing.join(", "))
            };
            return Ok(Box::new(NoticeBackend::new(backend, notice_line(&text))));
        }
        let windows = match read_pod(&reader, pod).await {
            Ok(resource) => PodContainers::of(&resource).is_windows(),
            Err(_) => false,
        };
        Err(no_shell(&missing, container.as_deref(), windows))
    }

    /// Attaches to the main process of `container` of `pod` (`None`: the default container).
    ///
    /// # Errors
    ///
    /// As [`open_shell`](Self::open_shell), without the shell search.
    pub async fn attach(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        let (port, reader) = self.ports(&pod.cluster)?;
        let container = self.resolve(&reader, pod, container).await?;
        // The terminal of a debug container (E09-S10) claims the session that created it.
        if let Some(opened) = container
            .as_deref()
            .and_then(|name| self.debug.claim(pod, name))
        {
            return Ok(opened);
        }
        let mut target = AttachTarget::interactive(pod.clone());
        target.container = container.as_deref().map(str::to_owned);
        let backend = port.attach(&target).await.map_err(explain_open)?;
        let text = format!(
            "attached to {} (its main process)",
            place(pod, container.as_deref())
        );
        Ok(Box::new(NoticeBackend::new(backend, notice_line(&text))))
    }

    /// Runs `command` (argv, no shell) in `container` of `pod` with a TTY and stdin.
    ///
    /// # Errors
    ///
    /// `Validation` for an empty command; otherwise as [`open_shell`](Self::open_shell).
    pub async fn exec(
        &self,
        pod: &ResourceRef,
        container: Option<&str>,
        command: &[String],
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        if command
            .first()
            .is_none_or(|program| program.trim().is_empty())
        {
            return Err(OxiError::validation("there is no command to run"));
        }
        let (port, reader) = self.ports(&pod.cluster)?;
        let container = self.resolve(&reader, pod, container).await?;
        let mut target = ExecTarget::interactive(pod.clone(), command.to_vec());
        target.container = container.as_deref().map(str::to_owned);
        port.exec(&target).await.map_err(explain_open)
    }

    /// The exec port and the pod reader of `cluster`'s connection.
    pub(super) fn ports(
        &self,
        cluster: &ClusterId,
    ) -> OxiResult<(Arc<dyn ExecPort>, Arc<dyn ResourceReader>)> {
        let session = self
            .sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        let not_connected =
            || OxiError::conflict("the cluster is not connected").with_retryable(true);
        let exec = session.exec().ok_or_else(not_connected)?;
        let reader = session.resources().ok_or_else(not_connected)?;
        Ok((exec, reader))
    }

    /// The container a session opens: the one named, else the pod's default, remembered for the
    /// picker. `None` when the pod cannot be read (an account allowed to exec but not to get
    /// pods): the API server then picks.
    async fn resolve(
        &self,
        reader: &Arc<dyn ResourceReader>,
        pod: &ResourceRef,
        requested: Option<&str>,
    ) -> OxiResult<Option<Arc<str>>> {
        let chosen = match requested {
            Some(name) => Some(Arc::from(name)),
            None => match read_pod(reader, pod).await {
                Ok(resource) => Some(container_to_open(&PodContainers::of(&resource), None)?),
                Err(error) if error.kind() == ErrorKind::NotFound => return Err(error),
                Err(error) => {
                    tracing::debug!(kind = ?error.kind(), "could not read the pod to choose its container");
                    None
                }
            },
        };
        if let Some(name) = &chosen {
            self.remember(pod, name);
        }
        Ok(chosen)
    }
}

pub(super) async fn read_pod(
    reader: &Arc<dyn ResourceReader>,
    pod: &ResourceRef,
) -> OxiResult<Resource> {
    reader.get(&pod.gvk, pod.namespace(), &pod.name).await
}

/// `web-0/app`, or `web-0` when the container is not known.
fn place(pod: &ResourceRef, container: Option<&str>) -> String {
    match container {
        Some(container) => format!("{}/{container}", pod.name),
        None => pod.name.to_string(),
    }
}
