//! [`RowActions`]: the row actions the bus can run, cached, and their resolution per kind.

use std::sync::Arc;

use oxikube_domain::Capabilities;
use oxikube_domain::command::{Command, CommandMeta};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::kinds::ResourceKind;

use super::registry::{KindFilter, RowActionRegistry};
use super::state::{ActionContext, ActionState};
use crate::CommandBus;

/// One row action: a registered command with its declared metadata.
#[derive(Clone, Copy, Debug)]
pub struct RowAction {
    meta: CommandMeta,
    label: &'static str,
    kinds: KindFilter,
    build: fn(&ResourceRef) -> Command,
    bulk: bool,
    order: u16,
}

impl RowAction {
    /// The command's metadata (title, scope, mutating, confirmation tier, capabilities).
    pub fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    /// The menu label.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The command for `target`.
    pub fn command_for(&self, target: &ResourceRef) -> Command {
        (self.build)(target)
    }

    /// Whether the action applies to `kind` with `capabilities` (it is offered at all).
    pub fn applies_to(&self, kind: &ResourceKind, capabilities: Capabilities) -> bool {
        self.kinds.accepts(kind) && self.meta.needs.satisfied_by(capabilities)
    }
}

/// A [`RowAction`] with its state in one session: what a menu or the palette draws.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedAction {
    /// The action.
    pub action: RowAction,
    /// Whether it can run now.
    pub state: ActionState,
}

/// The row actions of one `CommandBus`, joined once. Cheap to clone. See the
/// [module docs](super).
#[derive(Clone, Debug)]
pub struct RowActions {
    actions: Arc<[RowAction]>,
}

impl RowActions {
    /// Joins `registry` with `bus`: an action is kept when its command is registered on the bus
    /// (an action nobody handles is never offered), with the metadata the bus holds for it. Call
    /// it once after the bus is built; a menu then never asks the bus.
    pub fn from_bus(bus: &CommandBus, registry: &RowActionRegistry) -> Self {
        let mut actions: Vec<RowAction> = registry
            .specs()
            .iter()
            .filter_map(|spec| {
                let meta = *bus.get(spec.command)?.meta();
                Some(RowAction {
                    meta,
                    label: spec.label.unwrap_or(meta.title),
                    kinds: spec.kinds,
                    build: spec.build,
                    bulk: spec.bulk,
                    order: spec.order,
                })
            })
            .collect();
        // Stable: ties keep registration order.
        actions.sort_by_key(|action| action.order);
        Self {
            actions: actions.into(),
        }
    }

    /// The actions that apply to `kind` and that a session with `capabilities` can ever run, in
    /// menu order. An action the session lacks the capability for is left out.
    pub fn actions_for(&self, kind: &ResourceKind, capabilities: Capabilities) -> Vec<RowAction> {
        self.actions
            .iter()
            .filter(|action| action.applies_to(kind, capabilities))
            .copied()
            .collect()
    }

    /// What a menu or the palette shows for `selected` objects of `kind` in a session of `ctx`:
    /// [`actions_for`](Self::actions_for), each with its [`ActionState`]. With several objects
    /// selected only the bulk actions remain.
    pub fn resolve(
        &self,
        kind: &ResourceKind,
        ctx: &ActionContext,
        selected: usize,
    ) -> Vec<ResolvedAction> {
        self.actions
            .iter()
            .filter(|action| {
                (selected <= 1 || action.bulk) && action.applies_to(kind, ctx.capabilities)
            })
            .map(|action| ResolvedAction {
                action: *action,
                state: ctx.state_of(&action.meta),
            })
            .collect()
    }
}
