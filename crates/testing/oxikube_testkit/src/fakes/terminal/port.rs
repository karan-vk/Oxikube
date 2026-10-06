//! [`FakeExecPort`].

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_ports::{
    AttachTarget, DebugContainerSpec, ExecPort, ExecTarget, NodeShellSpec, TerminalBackend,
};
use parking_lot::Mutex;

use super::FakeTerminalBackend;
use crate::script::{CallLog, Script};

/// Queued backends for each [`FakeExecPort`] method. Push a [`FakeTerminalBackend`] clone
/// (to keep a handle on it) or an error.
#[derive(Debug, Default)]
pub struct ExecPortScripts {
    /// `exec`.
    pub exec: Script<FakeTerminalBackend>,
    /// `attach`.
    pub attach: Script<FakeTerminalBackend>,
    /// `create_debug_container`.
    pub create_debug_container: Script<FakeTerminalBackend>,
    /// `node_shell`.
    pub node_shell: Script<FakeTerminalBackend>,
}

/// One call made on a [`FakeExecPort`], with the descriptor it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecPortCall {
    /// `exec(target)`.
    Exec(ExecTarget),
    /// `attach(target)`.
    Attach(AttachTarget),
    /// `create_debug_container(spec)`.
    CreateDebugContainer(DebugContainerSpec),
    /// `node_shell(spec)`.
    NodeShell(NodeShellSpec),
}

/// Fake `ExecPort`. Each method records its descriptor and returns the next queued
/// [`FakeTerminalBackend`]; with nothing queued it returns a fresh echoing backend, kept
/// in [`opened`](Self::opened) so a test can inspect it. A queued error fails the call.
#[derive(Debug, Default)]
pub struct FakeExecPort {
    script: ExecPortScripts,
    calls: CallLog<ExecPortCall>,
    opened: Mutex<Vec<FakeTerminalBackend>>,
}

fake_plumbing!(FakeExecPort, ExecPortScripts, ExecPortCall);

impl FakeExecPort {
    /// A fake with nothing scripted: every call opens an echoing backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every backend handed out so far (scripted or fallback), in order.
    pub fn opened(&self) -> Vec<FakeTerminalBackend> {
        self.opened.lock().clone()
    }

    fn hand_out(
        &self,
        scripted: &Script<FakeTerminalBackend>,
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        let backend = scripted.next_or_else(|| Ok(FakeTerminalBackend::echo()))?;
        self.opened.lock().push(backend.clone());
        Ok(Box::new(backend))
    }
}

#[async_trait]
impl ExecPort for FakeExecPort {
    async fn exec(&self, target: &ExecTarget) -> OxiResult<Box<dyn TerminalBackend>> {
        self.calls.record(ExecPortCall::Exec(target.clone()));
        self.hand_out(&self.script.exec)
    }

    async fn attach(&self, target: &AttachTarget) -> OxiResult<Box<dyn TerminalBackend>> {
        self.calls.record(ExecPortCall::Attach(target.clone()));
        self.hand_out(&self.script.attach)
    }

    async fn create_debug_container(
        &self,
        spec: &DebugContainerSpec,
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        self.calls
            .record(ExecPortCall::CreateDebugContainer(spec.clone()));
        self.hand_out(&self.script.create_debug_container)
    }

    async fn node_shell(&self, spec: &NodeShellSpec) -> OxiResult<Box<dyn TerminalBackend>> {
        self.calls.record(ExecPortCall::NodeShell(spec.clone()));
        self.hand_out(&self.script.node_shell)
    }
}
