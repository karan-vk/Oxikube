//! [`CommandMeta`]: the static description of a command.

use serde::Serialize;

use super::capability::Capabilities;
use super::id::CommandId;
use crate::safety::{ConfirmTier, Risk};

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
        }
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
        }
    }
}

const _: () = {
    const fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<CommandMeta>();
};
