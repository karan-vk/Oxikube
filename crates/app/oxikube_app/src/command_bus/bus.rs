//! [`CommandBus`]: routes a [`Command`] to its handler, through the guard when it mutates.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use oxikube_domain::command::{Command, CommandId, CommandMeta};
use oxikube_ports::ToolDef;

use super::context::{DispatchContext, Outcome};
use super::error::DispatchError;
use super::handler::HandlerContext;
use super::registry::{CommandRegistry, Registered};
use crate::guard::{ConfirmationToken, MutationGuard, policy};

/// Dispatches every user and agent action. See the [module docs](super).
///
/// Cheap to clone; clones share the handlers and the guard. The handler table is frozen
/// when the bus is built, so a dispatch takes no lock to find its handler.
#[derive(Clone)]
pub struct CommandBus {
    inner: Arc<Inner>,
}

struct Inner {
    entries: IndexMap<CommandId, Registered>,
    guard: MutationGuard,
}

impl fmt::Debug for CommandBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandBus")
            .field("commands", &self.inner.entries.len())
            .finish_non_exhaustive()
    }
}

impl CommandBus {
    /// A bus over every handler in `registry`, guarding mutations with `guard`.
    pub fn new(registry: CommandRegistry, guard: MutationGuard) -> Self {
        Self {
            inner: Arc::new(Inner {
                entries: registry.entries,
                guard,
            }),
        }
    }

    /// Runs `command`.
    ///
    /// 1. The handler is looked up by [`Command::id`]; an unregistered id is
    ///    [`DispatchError::UnknownCommand`].
    /// 2. A privileged command (read-only toggle) is refused for agents and plugins.
    /// 3. A posture command (read-only mode, colour, preset) runs the guard's posture
    ///    pipeline: a simple confirm when it lifts read-only on a production-flagged
    ///    cluster, then the handler, then an audit record. Read-only mode does not block it.
    /// 4. A read command runs its handler at once, with no [`Mutation`](crate::guard::Mutation).
    /// 5. An exec-class command (`pod::Shell`, `pod::Attach`, `pod::Exec`) takes the guard's exec
    ///    policy: read-only block unless the cluster allows it, then an audit record; no
    ///    confirmation.
    /// 6. A mutating command goes through the [`MutationGuard`] pipeline: read-only
    ///    check, confirmation (returning [`Outcome::NeedsConfirmation`] without waiting),
    ///    dry-run stage, the handler with a `Mutation`, then the audit record.
    ///
    /// Steps 1 to 4's checks are synchronous and in memory; the first `.await` is the
    /// audit write or the handler.
    ///
    /// # Errors
    ///
    /// See [`DispatchError`].
    pub async fn dispatch(
        &self,
        command: Command,
        ctx: DispatchContext,
    ) -> Result<Outcome, DispatchError> {
        let id = command.id();
        let entry = self
            .inner
            .entries
            .get(&id)
            .ok_or(DispatchError::UnknownCommand(id))?;
        // The declaration, not anything a handler supplied, decides how the command is
        // guarded (registration also checks they are equal).
        let meta = command.meta();
        tracing::debug!(command = %id, initiator = %ctx.initiator, "dispatch");
        if !meta.allows(ctx.initiator) {
            return Err(DispatchError::NotPermitted {
                command: id,
                initiator: ctx.initiator,
            });
        }
        if meta.exec {
            // A shell, attach or exec in a container: read-only block (unless the cluster allows
            // it) and an audit record, no confirmation and no write permit.
            return self
                .inner
                .guard
                .run_exec(meta, command, ctx, entry.handler.clone())
                .await;
        }
        if !meta.mutating && policy::is_posture(&command) {
            // Posture commands (read-only, colour, presets) never touch the cluster, so the
            // read-only check does not apply, but they confirm and audit like a mutation.
            return self
                .inner
                .guard
                .run_posture(meta, command, ctx, entry.handler.clone())
                .await;
        }
        if !meta.mutating {
            let cluster = policy::cluster_of(&command).cloned().or(ctx.cluster);
            let cx = HandlerContext::new(ctx.initiator, ctx.who, cluster, None);
            return entry
                .handler
                .handle(command, cx)
                .await
                .map(Outcome::Completed)
                .map_err(DispatchError::Handler);
        }
        self.inner
            .guard
            .run(meta, command, ctx, entry.handler.clone())
            .await
    }

    /// Declines a pending confirmation: the request is forgotten and an audit record
    /// with outcome `Cancelled` is written.
    ///
    /// # Errors
    ///
    /// [`DispatchError::Confirmation`] for an unknown token, or
    /// [`DispatchError::AuditFailed`] when the record could not be written.
    pub async fn decline(&self, token: ConfirmationToken) -> Result<(), DispatchError> {
        self.inner.guard.decline(token).await
    }

    /// Whether `id` has a handler.
    pub fn is_registered(&self, id: CommandId) -> bool {
        self.inner.entries.contains_key(&id)
    }

    /// The registered commands' metadata, in registration order (for the palette).
    pub fn commands(&self) -> impl Iterator<Item = &CommandMeta> {
        self.inner.entries.values().map(|e| &e.meta)
    }

    /// The crate that registered `id`.
    pub fn owner(&self, id: CommandId) -> Option<&'static str> {
        self.inner.entries.get(&id).map(|e| e.owner)
    }

    /// The MCP tool stub of `id` (`None` for unregistered or privileged commands).
    pub fn tool(&self, id: CommandId) -> Option<&ToolDef> {
        self.inner.entries.get(&id).and_then(|e| e.tool.as_ref())
    }

    /// Every registered tool stub, for the `ToolRegistry` (agent phase).
    pub fn tools(&self) -> impl Iterator<Item = &ToolDef> {
        self.inner.entries.values().filter_map(|e| e.tool.as_ref())
    }

    /// The tool stubs agents are offered: every one, except those hidden from agents by default
    /// (the exec tools: `k8s.pod_shell`, `k8s.pod_attach`, `k8s.pod_exec`, unsafe and
    /// interactive) unless `include_hidden` is set, which the agent epic wires to the user's
    /// opt-in setting.
    pub fn agent_tools(&self, include_hidden: bool) -> impl Iterator<Item = &ToolDef> {
        self.tools()
            .filter(move |tool| include_hidden || tool.agent_exposed_by_default())
    }

    /// The guard, for its state (pending confirmations, audit backlog).
    pub fn guard(&self) -> &MutationGuard {
        &self.inner.guard
    }
}
