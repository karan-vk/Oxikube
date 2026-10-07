//! [`AuditedBackend`]: a node shell's terminal, whose end is the second audit record.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::OxiResult;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::{BackendEvent, NodeShellSpec, TerminalBackend, TerminalSize};

use crate::audit::AuditLog;
use crate::guard::policy::node_shell_detail;

/// What the closing record says; written once, when the shell ends or its backend is dropped.
///
/// The record says the shell ended and its pod's deletion was requested (the adapter deletes the
/// pod as the session ends and logs a delete that failed; the sweep and the pod's deadline are
/// the backstop), so its outcome is `Succeeded` whatever became of that delete.
pub(super) struct CloseAudit {
    log: Arc<AuditLog>,
    who: String,
    initiator: Initiator,
    node: ResourceRef,
    spec: NodeShellSpec,
    written: AtomicBool,
}

impl CloseAudit {
    pub(super) fn new(
        log: Arc<AuditLog>,
        who: String,
        initiator: Initiator,
        node: ResourceRef,
        spec: NodeShellSpec,
    ) -> Arc<Self> {
        Arc::new(Self {
            log,
            who,
            initiator,
            node,
            spec,
            written: AtomicBool::new(false),
        })
    }

    fn record(&self) -> Option<AuditRecord> {
        if self.written.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(self.log.entry_with_detail(
            &self.who,
            self.initiator,
            CommandId::NODE_SHELL.as_str(),
            self.node.clone(),
            &node_shell_detail("delete", &self.spec),
            AuditOutcome::Succeeded,
        ))
    }

    /// Whether the closing record was queued already.
    pub(super) fn is_written(&self) -> bool {
        self.written.load(Ordering::SeqCst)
    }

    /// Queues the record and writes it. A failed write stays queued: the next guarded command
    /// retries it.
    pub(super) async fn finish(&self) {
        if let Some(record) = self.record()
            && let Err(error) = self.log.record(record).await
        {
            tracing::warn!(kind = ?error.kind(), "could not audit the end of a node shell");
        }
    }

    /// Queues the record without waiting (`Drop`): written with the next flush, which the app's
    /// quit makes ([`ExecService::close_node_shells`](crate::ExecService::close_node_shells)).
    pub(super) fn queue(&self) {
        if let Some(record) = self.record() {
            drop(self.log.record(record));
        }
    }
}

/// The node shell's backend: everything is the wrapped backend's, and the end of the session (an
/// exit, a failure, a kill, or the backend being dropped when its tab closes) writes the
/// `phase=delete` audit record once.
pub(super) struct AuditedBackend {
    inner: Box<dyn TerminalBackend>,
    close: Arc<CloseAudit>,
}

impl AuditedBackend {
    pub(super) fn new(inner: Box<dyn TerminalBackend>, close: Arc<CloseAudit>) -> Self {
        Self { inner, close }
    }
}

impl Drop for AuditedBackend {
    fn drop(&mut self) {
        self.close.queue();
    }
}

#[async_trait]
impl TerminalBackend for AuditedBackend {
    async fn write(&self, bytes: &[u8]) -> OxiResult<()> {
        self.inner.write(bytes).await
    }

    async fn resize(&self, size: TerminalSize) -> OxiResult<()> {
        self.inner.resize(size).await
    }

    fn output_stream(&self) -> BoxStream<'static, BackendEvent> {
        let close = self.close.clone();
        self.inner
            .output_stream()
            .then(move |event| {
                let close = close.clone();
                async move {
                    if matches!(event, BackendEvent::Exited(_) | BackendEvent::Error(_)) {
                        close.finish().await;
                    }
                    event
                }
            })
            .boxed()
    }

    async fn kill(&self) -> OxiResult<()> {
        let killed = self.inner.kill().await;
        self.close.finish().await;
        killed
    }

    fn working_directory(&self) -> Option<PathBuf> {
        self.inner.working_directory()
    }
}
