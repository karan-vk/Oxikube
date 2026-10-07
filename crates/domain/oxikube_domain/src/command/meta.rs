//! [`CommandMeta`]: the static description of a command.

use serde::Serialize;

use super::capability::Capabilities;
use super::id::CommandId;
use crate::safety::{ConfirmTier, Initiator, Risk};

/// What a command acts on. Drives where the palette offers it and which
/// context it needs before it can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandScope {
    /// Always available (open a view, toggle the palette).
    Global,
    /// Needs an active cluster.
    Cluster,
    /// Needs an active cluster and a namespace selection.
    Namespace,
    /// Needs a resource kind (a list view of that kind).
    ResourceKind,
    /// Needs a selected resource (a [`ResourceRef`](crate::ids::ResourceRef)).
    Selection,
}

/// Static metadata of a command. Every field is `'static`-friendly, so a
/// registry is a `static` slice and palette filtering never allocates.
///
/// The guard derives its behaviour from this: `mutating` commands are blocked
/// on read-only clusters and must carry a `risk`; `confirm` is the default
/// confirmation tier (the guard may raise it for a sensitive target, never
/// lower it); `needs` lets the palette hide commands a session cannot run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CommandMeta {
    /// The command's id (also its keymap action name).
    pub id: CommandId,
    /// Human title for the palette and menus.
    pub title: &'static str,
    /// What the command acts on.
    pub scope: CommandScope,
    /// Whether the command changes cluster state (goes through `MutationGuard`).
    pub mutating: bool,
    /// Default confirmation tier; [`ConfirmTier::None`] for non-mutating commands.
    pub confirm: ConfirmTier,
    /// Capabilities the session or backend must have.
    pub needs: Capabilities,
    /// Blast radius; `Some` exactly when `mutating`.
    pub risk: Option<Risk>,
    /// Changes the safety posture itself (for example lifting read-only mode).
    /// Such a command is not `mutating` (it must stay runnable on a read-only
    /// cluster) but only a human may run it: see [`CommandMeta::allows`].
    pub privileged: bool,
    /// Opens an interactive session in a container (`pod::Shell`, `pod::Attach`, `pod::Exec`):
    /// an *exec-class* command. It is not `mutating` (it asks for no confirmation and names no
    /// risk), but it is as dangerous as a shell is, so the guard blocks it on a read-only
    /// cluster unless the cluster's `exec_in_read_only` setting allows it, audits every open,
    /// and its MCP tool stub is unsafe, interactive and hidden from agents by default
    /// ([`CommandMeta::tool_risk`]).
    pub exec: bool,
    /// Ends in an interactive terminal session in a container: every exec-class command, and
    /// `pod::Debug` (a mutation that adds a debug container and opens a terminal in it). What a
    /// user types next cannot be described by a schema, so the MCP tool stub is unsafe,
    /// interactive and hidden from agents by default.
    pub interactive: bool,
}

impl CommandMeta {
    /// A read-only or UI-local command: no confirmation, no risk.
    pub const fn read(
        id: CommandId,
        title: &'static str,
        scope: CommandScope,
        needs: Capabilities,
    ) -> Self {
        Self {
            id,
            title,
            scope,
            mutating: false,
            confirm: ConfirmTier::None,
            needs,
            risk: None,
            privileged: false,
            exec: false,
            interactive: false,
        }
    }

    /// An exec-class command: it opens a shell, an attach or a command in a container. It
    /// needs [`Capabilities::EXEC`], asks no confirmation, and is not `mutating` (it changes no
    /// object), but the guard treats it as its own class (see [`CommandMeta::exec`]).
    pub const fn exec(id: CommandId, title: &'static str, scope: CommandScope) -> Self {
        Self {
            exec: true,
            interactive: true,
            ..Self::read(id, title, scope, Capabilities::EXEC)
        }
    }

    /// A mutation that ends in an interactive terminal session (`pod::Debug`: it adds an
    /// ephemeral container to a pod and opens a terminal in it). A [`mutation`](Self::mutation)
    /// of `risk` that also needs [`Capabilities::EXEC`]; it is not exec-class
    /// ([`CommandMeta::exec`] stays `false`: the mutation pipeline, with its confirmation, applies),
    /// but its tool stub is unsafe, interactive and hidden from agents like theirs.
    pub const fn interactive_mutation(
        id: CommandId,
        title: &'static str,
        scope: CommandScope,
        risk: Risk,
    ) -> Self {
        Self {
            interactive: true,
            ..Self::mutation(id, title, scope, risk, Capabilities::EXEC)
        }
    }

    /// A non-mutating command that changes the safety posture (read-only
    /// mode). It is `privileged`: the guard refuses it for agents and plugins
    /// (ADR 0012: agents cannot bypass the UI's safety).
    pub const fn privileged(
        id: CommandId,
        title: &'static str,
        scope: CommandScope,
        needs: Capabilities,
    ) -> Self {
        Self {
            privileged: true,
            ..Self::read(id, title, scope, needs)
        }
    }

    /// Whether `initiator` may run this command at all. Privileged commands
    /// are limited to the UI and the command bus; everything else is open
    /// (mutations are still gated by the guard's own checks).
    pub const fn allows(&self, initiator: Initiator) -> bool {
        !self.privileged || matches!(initiator, Initiator::Ui | Initiator::Command)
    }

    /// A mutating command. The confirmation tier follows `risk`
    /// ([`Risk::confirm_tier`]) and [`Capabilities::MUTATE`] is always required.
    pub const fn mutation(
        id: CommandId,
        title: &'static str,
        scope: CommandScope,
        risk: Risk,
        needs: Capabilities,
    ) -> Self {
        Self {
            id,
            title,
            scope,
            mutating: true,
            confirm: risk.confirm_tier(),
            needs: needs.union(Capabilities::MUTATE),
            risk: Some(risk),
            privileged: false,
            exec: false,
            interactive: false,
        }
    }
}

const _: () = {
    const fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<CommandMeta>();
};
