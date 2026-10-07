//! [`ExecFlow`]: from "Shell" on a pod to the `pod::Shell` command with its container chosen.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Task, WeakEntity, Window};
use oxikube_app::{ContainerChoices, ContainerPlan, ExecService};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_runtime::spawn_kube;
use oxikube_workspace::{CommandDispatcher, Workspace};

use super::picker::ContainerPicker;

/// The two ways into a container from a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecKind {
    /// An interactive shell (`pod::Shell`).
    Shell,
    /// The container's main process (`pod::Attach`).
    Attach,
}

impl ExecKind {
    /// The kind a row action's command is, if it is one of the two.
    pub fn of(command: CommandId) -> Option<Self> {
        match command {
            CommandId::POD_SHELL => Some(Self::Shell),
            CommandId::POD_ATTACH => Some(Self::Attach),
            _ => None,
        }
    }

    /// The command that opens a session of this kind in `container` of `target`.
    pub fn command(self, target: ResourceRef, container: Option<String>) -> Command {
        match self {
            Self::Shell => Command::PodShell { target, container },
            Self::Attach => Command::PodAttach { target, container },
        }
    }

    /// What the picker's title says: "Open a shell in", "Attach to".
    pub fn verb(self) -> &'static str {
        match self {
            Self::Shell => "Open a shell in",
            Self::Attach => "Attach to",
        }
    }
}

/// Joins a pod's "Shell" / "Attach" to its command: reads the pod off the UI thread, then
/// dispatches the command with the container chosen, asking first (a picker in the cluster tab's
/// workspace) when the pod has several. Cheap to clone. See the [module docs](super).
#[derive(Clone)]
pub struct ExecFlow {
    service: Arc<ExecService>,
    dispatcher: Rc<dyn CommandDispatcher>,
    workspace: WeakEntity<Workspace>,
}

impl ExecFlow {
    /// A flow over `service` that dispatches on `dispatcher` and asks in `workspace`.
    pub fn new(
        service: Arc<ExecService>,
        dispatcher: Rc<dyn CommandDispatcher>,
        workspace: WeakEntity<Workspace>,
    ) -> Self {
        Self {
            service,
            dispatcher,
            workspace,
        }
    }

    /// Starts opening a session of `kind` in `target`. Returns the task that reads the pod and
    /// then dispatches or asks; the caller keeps it (dropping it cancels the read, so a view that
    /// closes while it runs leaves nothing behind).
    ///
    /// A pod that cannot be read (no `get`, not found) is not an error here: the command is
    /// dispatched without a container and the terminal says why it could not open.
    pub fn begin<V: 'static>(
        &self,
        kind: ExecKind,
        target: ResourceRef,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Task<()> {
        let service = self.service.clone();
        let planned = {
            let target = target.clone();
            spawn_kube(cx, async move { service.plan(&target, None).await })
        };
        let flow = self.clone();
        cx.spawn_in(window, async move |_, cx| {
            let plan = match planned.await {
                Ok(plan) => plan,
                Err(error) => Err(OxiError::from(error)),
            };
            cx.update(|window, cx| flow.resolve(kind, target, plan, window, cx))
                .ok();
        })
    }

    /// What the plan means: open at once, ask, or (the pod could not be read) let the open fail
    /// in the terminal.
    fn resolve(
        &self,
        kind: ExecKind,
        target: ResourceRef,
        plan: OxiResult<ContainerPlan>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match plan {
            Ok(ContainerPlan::Open(container)) => {
                self.dispatch(kind, target, Some(container.to_string()), cx);
            }
            Ok(ContainerPlan::Pick(choices)) => self.ask(kind, target, choices, window, cx),
            Err(error) => {
                tracing::debug!(kind = ?error.kind(), "could not read the pod to choose a container");
                self.dispatch(kind, target, None, cx);
            }
        }
    }

    /// Sends the command for `container` of `target` (`None`: the pod's default).
    pub fn dispatch(
        &self,
        kind: ExecKind,
        target: ResourceRef,
        container: Option<String>,
        cx: &mut App,
    ) {
        self.dispatcher
            .dispatch(kind.command(target, container), cx);
    }

    /// Opens the picker in the workspace; without one, takes the preselected container.
    fn ask(
        &self,
        kind: ExecKind,
        target: ResourceRef,
        choices: ContainerChoices,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            let default = choices.preselected().name.to_string();
            self.dispatch(kind, target, Some(default), cx);
            return;
        };
        let flow = self.clone();
        workspace.update(cx, |workspace, cx| {
            let picker = cx.new(|cx| ContainerPicker::new(kind, target, choices, flow, cx));
            workspace.show_modal(picker.clone(), window, cx);
            picker.update(cx, |picker, cx| picker.focus(window, cx));
        });
    }
}
