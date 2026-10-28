//! [`ResourceActions`]: the app's row actions and delete flow, as the tables use them.

use std::sync::Arc;

use oxikube_app::access::AccessRegistry;
use oxikube_app::namespaces::NamespaceCreator;
use oxikube_app::{
    ActionAccess, ActionContext, ActionState, ClusterSessionManager, CommandBus, DebugRunner,
    DeleteFlow, ExecService, RollbackRunner, RolloutService, RowAction, RowActionRegistry,
    RowActions, ScaleRunner, SelectionFlow, SetImageRunner, WorkloadService,
};
use oxikube_domain::Capabilities;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::kinds::ResourceKind;

use crate::kinds::storage::StorageFlows;

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
    exec: Option<Arc<ExecService>>,
    /// Runs `pod::Debug` for the debug dialog (E09-S10).
    debug: DebugRunner,
    /// Reads a workload's scale and runs `workload::Scale` for the scale dialog (E12-S04).
    workloads: Arc<WorkloadService>,
    scale: ScaleRunner,
    /// Runs `namespace::Create` for the create dialog (E12-S16).
    namespace_creator: NamespaceCreator,
    /// The flows of the storage dialogs: PVC resize, set the default StorageClass (E12-S15).
    storage: StorageFlows,
    /// Runs `workload::SetImage` for the set-image dialog (E12-S05).
    set_image: SetImageRunner,
    /// Reads a workload's rollout history and runs `workload::Rollback` for the rollback dialog
    /// (E12-S06).
    rollouts: Arc<RolloutService>,
    rollback: RollbackRunner,
    /// The drain dialog's flow (E12-S10); `None` until the app gives one.
    drain: Option<oxikube_app::nodes::DrainFlow>,
    /// Plans and runs the bulk commands for the bulk dialog (E12-S19).
    selection: SelectionFlow,
    /// Starts `portforward::Start` for the start dialog (E15-S04); `None` until the app has a
    /// port-forward manager.
    forward: Option<oxikube_app::ForwardStarter>,
    /// The cached access reviews that grey out actions the user may not run (E17-S03); `None`
    /// until the app gives them.
    access: Option<AccessRegistry>,
}

impl ResourceActions {
    /// The core row actions (delete), the pods' (E12-S03) and the CRD list's (E07-S07: open its custom
    /// resources, show its details) over `bus`, acting as `who` (the local user's name, for the audit log).
    ///
    /// # Panics
    ///
    /// If a CRD or pod row action has the id of one already registered: a programming error
    /// that the crate's tests catch.
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
        // A pod's delete is the pod kind's (`pod::Delete`, E12-S03), not the core one.
        for spec in crate::kinds::pods::row_actions() {
            registry
                .register(spec)
                .expect("the core registry has no pod action");
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
        let who: Arc<str> = who.into();
        Self {
            actions: RowActions::from_bus(bus, registry),
            flow: DeleteFlow::new(bus.clone(), sessions.clone(), who.clone()),
            sessions: sessions.clone(),
            exec: None,
            debug: DebugRunner::new(bus.clone(), who.clone()),
            workloads: Arc::new(WorkloadService::new(sessions.clone())),
            scale: ScaleRunner::new(bus.clone(), who.clone()),
            set_image: SetImageRunner::new(bus.clone(), who.clone()),
            rollback: RollbackRunner::new(bus.clone(), who.clone()),
            rollouts: Arc::new(RolloutService::new(sessions.clone())),
            selection: SelectionFlow::new(bus.clone(), sessions.clone(), who.clone()),
            storage: StorageFlows::new(bus, &sessions, &who),
            namespace_creator: NamespaceCreator::new(bus.clone(), who),
            drain: None,
            forward: None,
            access: None,
        }
    }

    /// Greys out a mutating action the cached access reviews of `access` say the user may not
    /// run ("You have no delete permission for this kind"). A soft gate: the guard and the API
    /// server still decide. Without it only the cluster's read-only mode greys actions out.
    #[must_use]
    pub fn with_access(mut self, access: AccessRegistry) -> Self {
        self.access = Some(access);
        self
    }

    /// Lets the Forward buttons, `shift-f` and the palette open the start dialog over `starter`
    /// (the app's port-forward manager on the same bus). Without it they say port forwarding is
    /// not available.
    #[must_use]
    pub fn with_forward(mut self, starter: oxikube_app::ForwardStarter) -> Self {
        self.forward = Some(starter);
        self
    }

    /// The flow that opens the port-forward start dialog in `workspace` (E15-S04): the Pod and
    /// Service panels' Forward buttons, `shift-f` and the palette. `None` without a starter.
    pub fn forward_flow(
        &self,
        workspace: gpui::WeakEntity<oxikube_workspace::Workspace>,
    ) -> Option<oxikube_portforward_ui::StartFlow> {
        let starter = self.forward.clone()?;
        Some(oxikube_portforward_ui::StartFlow::new(
            starter,
            self.sessions.clone(),
            workspace,
        ))
    }

    /// Lets "Drain…" open the drain dialog over `flow` (the app's `NodeService`, on the same
    /// bus). Without it the action says it is not available here.
    #[must_use]
    pub fn with_drain(mut self, flow: oxikube_app::nodes::DrainFlow) -> Self {
        self.drain = Some(flow);
        self
    }

    /// The drain dialog's flow, if one was given.
    pub fn drain_flow(&self) -> Option<&oxikube_app::nodes::DrainFlow> {
        self.drain.as_ref()
    }

    /// Deletes `n` objects at a time (at least one; the default is a few). One makes the requests
    /// follow the selection's order.
    #[must_use]
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.flow = self.flow.with_concurrency(n);
        self
    }

    /// Lets the pod actions choose a container before they dispatch: "Shell" and "Attach" read
    /// the pod through `service` and ask which container when it has several. Without it they
    /// dispatch at once and the pod's default container opens.
    #[must_use]
    pub fn with_exec(mut self, service: Arc<ExecService>) -> Self {
        self.exec = Some(service);
        self
    }

    /// The service the pod actions read pods through, if one was given.
    pub fn exec_service(&self) -> Option<&Arc<ExecService>> {
        self.exec.as_ref()
    }

    /// The runner of `pod::Debug` for the debug dialog: dispatches on the same bus, as the user.
    pub fn debug_runner(&self) -> DebugRunner {
        self.debug.clone()
    }

    /// The flow that opens the scale dialog in `workspace` (E12-S04): the table's "Scale" action
    /// and the workload panel's button.
    pub fn scale_flow(
        &self,
        workspace: gpui::WeakEntity<oxikube_workspace::Workspace>,
    ) -> crate::kinds::ScaleFlow {
        crate::kinds::ScaleFlow::new(self.workloads.clone(), self.scale.clone(), workspace)
    }

    /// The runner of `namespace::Create` for the create-namespace dialog: dispatches on the same
    /// bus, as the user.
    pub fn namespace_creator(&self) -> NamespaceCreator {
        self.namespace_creator.clone()
    }

    /// The flows behind the storage dialogs (resize a claim, set the default class).
    pub fn storage(&self) -> &StorageFlows {
        &self.storage
    }

    /// The flow that opens the set-image dialog in `workspace` (E12-S05): the table's "Set image"
    /// action and the detail header's button.
    pub fn set_image_flow(
        &self,
        workspace: gpui::WeakEntity<oxikube_workspace::Workspace>,
    ) -> crate::kinds::SetImageFlow {
        crate::kinds::SetImageFlow::new(self.workloads.clone(), self.set_image.clone(), workspace)
    }

    /// The flow that opens the rollback dialog in `workspace` (E12-S06): the table's "Rollback"
    /// action and the workload panel's button.
    pub fn rollback_flow(
        &self,
        workspace: gpui::WeakEntity<oxikube_workspace::Workspace>,
    ) -> crate::kinds::RollbackFlow {
        crate::kinds::RollbackFlow::new(self.rollouts.clone(), self.rollback.clone(), workspace)
    }

    /// The flow that opens the bulk dialog in `workspace` (E12-S19): delete, restart and edit
    /// labels of the table's selection.
    pub fn bulk_flow(
        &self,
        workspace: gpui::WeakEntity<oxikube_workspace::Workspace>,
    ) -> crate::bulk::BulkFlow {
        crate::bulk::BulkFlow::new(self.selection.clone(), workspace)
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

    /// Whether `command` is a row action of `kind` at all, whatever the session may do: the
    /// difference between "this kind has no such action" and "you are not allowed it here".
    pub(crate) fn offers(&self, kind: &ResourceKind, command: CommandId) -> bool {
        self.actions
            .actions_for(kind, Capabilities::all())
            .iter()
            .any(|action| action.meta().id == command)
    }

    /// The actions for `selected` objects of `kind` in `cluster`: what the context menu and the
    /// palette show. Without the objects' namespaces the RBAC gate judges by the namespaces the
    /// session has selected; prefer [`entries_for`](Self::entries_for).
    pub fn entries(
        &self,
        cluster: &ClusterId,
        kind: &ResourceKind,
        selected: usize,
    ) -> Vec<ActionEntry> {
        let namespaces = self
            .sessions
            .get(cluster)
            .map(|session| {
                session
                    .namespace_selection()
                    .names()
                    .map(|name| Some(name.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        self.entries_in(cluster, kind, selected, namespaces)
    }

    /// The actions for the objects `targets` of `kind`: [`entries`](Self::entries) with the RBAC
    /// gate judging each object's own namespace.
    pub fn entries_for(
        &self,
        cluster: &ClusterId,
        kind: &ResourceKind,
        targets: &[ResourceRef],
    ) -> Vec<ActionEntry> {
        let namespaces = targets
            .iter()
            .map(|target| target.namespace.as_deref().map(str::to_owned))
            .collect();
        self.entries_in(cluster, kind, targets.len(), namespaces)
    }

    fn entries_in(
        &self,
        cluster: &ClusterId,
        kind: &ResourceKind,
        selected: usize,
        namespaces: Vec<Option<String>>,
    ) -> Vec<ActionEntry> {
        let session = self.sessions.get(cluster);
        let ctx = session.as_ref().map(ActionContext::of).unwrap_or_default();
        let access = match (&self.access, &session) {
            (Some(registry), Some(session)) => registry
                .for_session(cluster, session)
                .map_or_else(ActionAccess::none, |service| {
                    ActionAccess::new(service, namespaces)
                }),
            _ => ActionAccess::none(),
        };
        self.actions
            .resolve_with(kind, &ctx, selected, &access)
            .into_iter()
            .map(|resolved| ActionEntry {
                label: label(&resolved.action, selected),
                action: resolved.action,
                state: resolved.state,
            })
            .collect()
    }
}

/// "Delete" for one object, "Delete 3 objects" for several (but "Sanitize" is "Sanitize").
fn label(action: &RowAction, selected: usize) -> String {
    // A sweep acts on the table's view, whatever is selected.
    if selected > 1 && action.meta().id != CommandId::POD_SANITIZE {
        format!("{} {selected} objects", action.label())
    } else {
        action.label().to_owned()
    }
}
