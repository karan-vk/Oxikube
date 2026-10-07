//! The node shell's two steps on [`ExecService`]: the guarded handler's
//! [`authorize_node_shell`](ExecService::authorize_node_shell) and the terminal's
//! [`open_node_shell`](ExecService::open_node_shell).

use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{NodeShellSpec, TerminalBackend, WriteOptions, node_shell_manifest};

use super::backend::{AuditedBackend, CloseAudit};
use super::failure::explain;
use super::permit::Permit;
use crate::command_bus::HandlerContext;
use crate::exec::notice::{NoticeBackend, notice_line};
use crate::exec::service::ExecService;

/// What a guarded node shell will create: shown in the toast of a dry run and tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeShellPlan {
    /// The node.
    pub node: String,
    /// The pod's image.
    pub image: String,
    /// The pod's namespace.
    pub namespace: String,
    /// Whether the dispatch was a dry run: the pod was validated by the server, nothing was
    /// created and no terminal will open.
    pub dry_run: bool,
}

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn node_name(target: &ResourceRef) -> OxiResult<&str> {
    if !target.gvk.is_node() || target.namespace.is_some() {
        return Err(OxiError::validation(format!(
            "node::Shell needs a node, not a {}",
            target.gvk.kind
        )));
    }
    Ok(&target.name)
}

impl ExecService {
    /// The guarded half of `node::Shell`, for the command's handler: validates `target`,
    /// renders the shell pod from the cluster's settings, asks the server to *dry-run* creating
    /// it through the command's [`Mutation`](crate::Mutation), and, unless the dispatch itself
    /// was a dry run, leaves the permit [`open_node_shell`](Self::open_node_shell) takes.
    ///
    /// The dry run is where a missing `create pods`, a pod security admission that refuses a
    /// privileged pod in the namespace, or a quota says so, with the namespace and the setting
    /// to change, before any terminal tab opens.
    ///
    /// # Errors
    ///
    /// `Validation` for a target that is not a node or a template that does not render (a blank
    /// image), `Internal` for a call without the guard's permit, `NotFound` for a cluster that is
    /// not open, and the server's refusal of the dry run (`Forbidden`, `Validation`, `Network`...)
    /// with advice added.
    pub async fn authorize_node_shell(
        &self,
        cx: &HandlerContext,
        target: &ResourceRef,
    ) -> OxiResult<NodeShellPlan> {
        let mutation = cx.require_mutation()?;
        let node = node_name(target)?;
        let spec = self.node_shell_spec(&target.cluster, node)?;
        let manifest = node_shell_manifest(&spec)?;
        mutation
            .writer()
            .create(
                &pod_gvk(),
                Some(&spec.namespace),
                &manifest,
                &WriteOptions::dry_run().manager("oxikube"),
            )
            .await
            .map_err(|error| explain(error, &spec))?;
        let plan = NodeShellPlan {
            node: node.to_owned(),
            image: spec.image.clone(),
            namespace: spec.namespace.clone(),
            dry_run: mutation.dry_run(),
        };
        if !plan.dry_run {
            self.permits
                .grant(target.clone(), spec, cx.who(), cx.initiator());
        }
        Ok(plan)
    }

    /// The template of a shell on `node` of `cluster`, from the cluster's settings.
    fn node_shell_spec(&self, cluster: &ClusterId, node: &str) -> OxiResult<NodeShellSpec> {
        let session = self
            .sessions
            .get(cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        Ok(NodeShellSpec::for_node(node, session.prefs()))
    }

    /// Ends every node shell that is still open, for the app's quit (GPUI does not drop the
    /// terminals then): has the adapters delete their pods and waits for them, writes the
    /// closing audit record of each shell and flushes the audit log (which also writes the
    /// records of tabs closed earlier that are still queued). Returns how many pods were deleted.
    /// Bounded by the adapters' delete timeouts, and the quit's own, so call it from an
    /// `on_app_quit` future. Safe to call twice.
    pub async fn close_node_shells(&self) -> usize {
        self.open_shells.close_all(self.audit.get()).await
    }

    /// Opens the shell a guarded `node::Shell` allowed: sweeps the cluster's leftover shell pods
    /// the first time (those whose owner stopped stamping them; audited), then has the adapter create the pod, wait for it, exec `nsenter` into the
    /// node's namespaces and hand back the session. The pod is deleted when the session ends,
    /// fails to open, or the returned backend is dropped (the tab closed); the end is audited.
    ///
    /// The terminal starts with one line saying which pod serves the shell.
    ///
    /// # Errors
    ///
    /// `Forbidden` when no guarded `node::Shell` allowed this shell (or its permit expired), so
    /// nothing is created; `Conflict` / `Timeout` (with the image and the settings to check) when
    /// the pod cannot start, `Forbidden` for RBAC, admission or quota, and the usual transport
    /// kinds. A pod that was created is deleted before the error returns.
    pub async fn open_node_shell(&self, node: &ResourceRef) -> OxiResult<Box<dyn TerminalBackend>> {
        let Permit {
            node,
            spec,
            who,
            initiator,
            ..
        } = self.permits.take(node).ok_or_else(|| {
            OxiError::forbidden(
                "a node shell opens only through the `node::Shell` command, which confirms and \
                 audits it; start it again from the node's menu",
            )
        })?;
        let (port, _) = self.ports(&node.cluster)?;
        self.sweep_once(&node.cluster, &*port, &spec.namespace, &who, initiator)
            .await;
        let backend = port
            .node_shell(&spec)
            .await
            .map_err(|error| explain(error, &spec))?;
        let notice = notice_line(&format!(
            "node shell on {}: privileged pod in {} ({}), deleted when this tab closes",
            spec.node, spec.namespace, spec.image
        ));
        let (backend, close): (Box<dyn TerminalBackend>, _) = match self.audit.get() {
            Some(log) => {
                let close = CloseAudit::new(log.clone(), who, initiator, node, spec);
                (
                    Box::new(AuditedBackend::new(backend, close.clone())),
                    Some(close),
                )
            }
            None => (backend, None),
        };
        self.open_shells.track(&port, close.as_ref());
        Ok(Box::new(NoticeBackend::new(backend, notice)))
    }
}
