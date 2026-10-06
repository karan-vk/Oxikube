//! [`ResourceActions`]: the app's row actions and delete flow, as the tables use them.

use oxikube_app::{
    ActionContext, ActionState, ClusterSessionManager, CommandBus, DeleteFlow, RowAction,
    RowActionRegistry, RowActions,
};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kinds::ResourceKind;

/// One action as a menu or the palette shows it: the action, its label for this many objects,
/// and whether it can run.
#[derive(Clone, Debug)]
pub struct ActionEntry {
    /// The action.
    pub action: RowAction,
    /// "Delete", or "Delete 3 objects" for a selection.
    pub label: String,
    /// Whether it can run now, and if not, why.
    pub state: ActionState,
}

impl ActionEntry {
    /// The command the action sends.
    pub fn command(&self) -> CommandId {
        self.action.meta().id
    }

    /// Whether it can run now.
    pub fn is_enabled(&self) -> bool {
        self.state.is_enabled()
    }

    /// The line that says why it is disabled.
    pub fn reason(&self) -> Option<String> {
        self.state.reason().map(|reason| reason.to_string())
    }
}

/// What every table needs for row actions. Cheap to clone; build it once per window, after the
/// `CommandBus`.
///
/// The [`RowActions`] snapshot is taken here, so opening a menu is a filter over a short list
/// and never asks the bus.
#[derive(Clone)]
pub struct ResourceActions {
    actions: RowActions,
    flow: DeleteFlow,
    sessions: ClusterSessionManager,
}

impl ResourceActions {
    /// The core row actions (delete) and the CRD list's (E07-S07: open its custom resources,
    /// show its details) over `bus`, acting as `who` (the local user's name, for the audit log).
    pub fn new(
        bus: &CommandBus,
        sessions: ClusterSessionManager,
        who: impl Into<std::sync::Arc<str>>,
    ) -> Self {
        let mut registry = RowActionRegistry::core();
        for spec in crate::crds::crd_row_actions() {
            registry
                .register(spec)
                .expect("the core registry has no CRD action");
        }
        Self::with_registry(bus, sessions, who, &registry)
    }

    /// Row actions of `registry` (the core ones plus what other crates registered) over `bus`.
    pub fn with_registry(
        bus: &CommandBus,
        sessions: ClusterSessionManager,
        who: impl Into<std::sync::Arc<str>>,
        registry: &RowActionRegistry,
    ) -> Self {
        Self {
            actions: RowActions::from_bus(bus, registry),
            flow: DeleteFlow::new(bus.clone(), sessions.clone(), who),
            sessions,
        }
    }

    /// Deletes `n` objects at a time (at least one; the default is a few). One makes the requests
    /// follow the selection's order.
    #[must_use]
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.flow = self.flow.with_concurrency(n);
        self
    }

    /// The delete flow.
    pub fn flow(&self) -> &DeleteFlow {
        &self.flow
    }

    /// What `cluster`'s session allows (nothing while it has none).
    pub fn context(&self, cluster: &ClusterId) -> ActionContext {
        self.sessions
            .get(cluster)
            .map(|session| ActionContext::of(&session))
            .unwrap_or_default()
    }

    /// The actions for `selected` objects of `kind` in `cluster`: what the context menu and the
    /// palette show.
    pub fn entries(
        &self,
        cluster: &ClusterId,
        kind: &ResourceKind,
        selected: usize,
    ) -> Vec<ActionEntry> {
        self.actions
            .resolve(kind, &self.context(cluster), selected)
            .into_iter()
            .map(|resolved| ActionEntry {
                label: label(&resolved.action, selected),
                action: resolved.action,
                state: resolved.state,
            })
            .collect()
    }
}

/// "Delete" for one object, "Delete 3 objects" for several.
fn label(action: &RowAction, selected: usize) -> String {
    if selected > 1 {
        format!("{} {selected} objects", action.label())
    } else {
        action.label().to_owned()
    }
}
