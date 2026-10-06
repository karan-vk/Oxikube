//! Test harness shared by the bus, guard and audit tests: a real
//! [`ClusterSessionManager`] over `oxikube_testkit` fakes, a [`FakeStatePort`] for the
//! audit log, and a [`CommandBus`] with recording test handlers. No runtime and no
//! threads: the fakes answer at once and futures are polled with `now_or_never`.

use std::sync::Arc;

use futures::FutureExt;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::command::{self, Command, CommandId, CommandMeta};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, DeleteOutcome, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeResourcePort,
    FakeStatePort, ResourceCall,
};
use parking_lot::Mutex;

use crate::command_bus::{
    CommandBus, CommandOutput, CommandRegistry, DispatchContext, DispatchError, HandlerContext,
    Outcome, RegisterError,
};
use crate::guard::{Confirmation, ConfirmationRequest, MutationGuard};
use crate::session::{ClusterSessionManager, SessionOptions};

/// The mutating commands the harness registers.
pub(crate) const MUTATING: [CommandId; 5] = [
    CommandId::NODE_DRAIN,
    CommandId::NODE_UNCORDON,
    CommandId::POD_DELETE,
    CommandId::RESOURCE_DELETE,
    CommandId::WORKLOAD_SCALE,
];

/// The read commands the harness registers.
pub(crate) const READS: [CommandId; 2] = [CommandId::POD_VIEW_LOGS, CommandId::CLUSTER_SELECT];

/// One handler call, as the test handlers saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub id: CommandId,
    pub initiator: Initiator,
    pub cluster: Option<ClusterId>,
    pub mutation: bool,
    pub dry_run: bool,
}

pub(crate) type Calls = Arc<Mutex<Vec<Call>>>;

pub(crate) fn declared(id: CommandId) -> CommandMeta {
    *command::lookup(id).expect("declared command")
}

pub(crate) fn cluster_context(name: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext {
        cluster: ClusterId::new("/home/me/.kube/config", &context),
        context,
        source: SourceId("kubeconfig".into()),
        server: Some(format!("https://{name}.example:6443")),
        default_namespace: None,
    }
}

pub(crate) fn id(name: &str) -> ClusterId {
    cluster_context(name).cluster
}

pub(crate) fn pod(cluster: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(id(cluster), Gvk::new("", "v1", "Pod"), "default", name)
}

pub(crate) fn node(cluster: &str, name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(id(cluster), Gvk::new("", "v1", "Node"), name)
}

pub(crate) fn pod_delete(cluster: &str, name: &str) -> Command {
    Command::PodDelete {
        target: pod(cluster, name),
        grace_period_seconds: None,
    }
}

pub(crate) fn view_logs(cluster: &str, name: &str) -> Command {
    Command::PodViewLogs {
        target: pod(cluster, name),
        container: None,
        follow: false,
        previous: false,
        tail_lines: None,
    }
}

pub(crate) fn ctx(initiator: Initiator) -> DispatchContext {
    DispatchContext::new(initiator, "alice")
}

fn record(calls: &Calls, cx: &HandlerContext, id: CommandId) {
    calls.lock().push(Call {
        id,
        initiator: cx.initiator(),
        cluster: cx.cluster().cloned(),
        mutation: cx.mutation().is_some(),
        dry_run: cx.mutation().is_some_and(|m| m.dry_run()),
    });
}

/// Registers the mutating test commands: each deletes its target through the permit.
pub(crate) fn register_mutations(
    reg: &mut CommandRegistry,
    calls: &Calls,
) -> Result<(), RegisterError> {
    for id in MUTATING {
        let calls = calls.clone();
        reg.register(declared(id), move |cmd: Command, cx: HandlerContext| {
            record(&calls, &cx, cmd.id());
            async move {
                let mutation = cx.require_mutation()?;
                let target = cmd.target().expect("test commands have a target");
                mutation
                    .writer()
                    .delete(
                        &target.gvk,
                        target.namespace(),
                        &target.name,
                        &mutation.delete_options(),
                    )
                    .await?;
                Ok(CommandOutput::message("done"))
            }
        })?;
    }
    Ok(())
}

/// Registers the read test commands and the privileged read-only toggle.
pub(crate) fn register_reads(
    reg: &mut CommandRegistry,
    calls: &Calls,
    manager: &ClusterSessionManager,
) -> Result<(), RegisterError> {
    for id in READS {
        let calls = calls.clone();
        reg.register(declared(id), move |cmd: Command, cx: HandlerContext| {
            record(&calls, &cx, cmd.id());
            async { Ok(CommandOutput::none()) }
        })?;
    }
    let (calls, manager) = (calls.clone(), manager.clone());
    reg.register(
        declared(CommandId::CLUSTER_TOGGLE_READ_ONLY),
        move |cmd: Command, cx: HandlerContext| {
            record(&calls, &cx, cmd.id());
            let result = match cmd {
                Command::ClusterToggleReadOnly { cluster, read_only } => {
                    let current = manager.get(&cluster).is_some_and(|s| s.read_only());
                    manager
                        .set_read_only(&cluster, read_only.unwrap_or(!current))
                        .map(|_| CommandOutput::none())
                }
                _ => Err(OxiError::internal("wrong command")),
            };
            async move { result }
        },
    )
}

/// A bus with the test handlers over sessions for contexts `a` and `b`.
pub(crate) struct Harness {
    pub manager: ClusterSessionManager,
    pub connector: Arc<FakeClusterConnectorPort>,
    pub state: Arc<FakeStatePort>,
    pub bus: CommandBus,
    pub calls: Calls,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_extra(|_, _| Ok(()))
    }

    /// The standard harness plus `node::Cordon`, whose handler sends its write and
    /// then never finishes (a dispatch that is still in flight when its caller gives
    /// up).
    pub fn with_hanging_cordon() -> Self {
        Self::with_extra(|reg, calls| {
            let calls = calls.clone();
            reg.register(
                declared(CommandId::NODE_CORDON),
                move |cmd: Command, cx: HandlerContext| {
                    record(&calls, &cx, cmd.id());
                    async move {
                        let mutation = cx.require_mutation()?;
                        let target = cmd.target().expect("cordon has a target");
                        mutation
                            .writer()
                            .delete(
                                &target.gvk,
                                target.namespace(),
                                &target.name,
                                &mutation.delete_options(),
                            )
                            .await?;
                        futures::future::pending::<()>().await;
                        Ok(CommandOutput::none())
                    }
                },
            )
        })
    }

    /// The standard harness plus the commands `extra` registers.
    fn with_extra(
        extra: impl FnOnce(&mut CommandRegistry, &Calls) -> Result<(), RegisterError>,
    ) -> Self {
        let connector = Arc::new(FakeClusterConnectorPort::new());
        let source = Arc::new(
            FakeClusterSourcePort::new()
                .with_contexts([cluster_context("a"), cluster_context("b")]),
        );
        let clock = Arc::new(FakeClockPort::default());
        let manager = ClusterSessionManager::new(connector.clone(), source, clock.clone());
        let state = Arc::new(FakeStatePort::new());
        let calls = Calls::default();

        let mut registry = CommandRegistry::new();
        registry
            .install("test_workloads", |reg| register_mutations(reg, &calls))
            .expect("mutations register");
        registry
            .install("test_views", |reg| register_reads(reg, &calls, &manager))
            .expect("reads register");
        registry
            .install("test_extra", |reg| extra(reg, &calls))
            .expect("extra commands register");
        let guard = MutationGuard::new(manager.clone(), state.clone(), clock);
        Self {
            bus: CommandBus::new(registry, guard),
            manager,
            connector,
            state,
            calls,
        }
    }

    /// Opens `name` with `read_only` and connects it.
    pub fn connect(&self, name: &str, read_only: bool) {
        self.manager.open(
            &cluster_context(name),
            SessionOptions {
                read_only,
                ..SessionOptions::default()
            },
        );
        self.manager
            .connect(&id(name))
            .now_or_never()
            .expect("connect does not wait")
            .expect("connect");
    }

    /// The fake resource port of `name`'s connection.
    pub fn resources(&self, name: &str) -> Arc<FakeResourcePort> {
        self.connector.ports_for(&id(name)).resources
    }

    /// Lets the next `n` deletes on `name` succeed.
    pub fn allow_deletes(&self, name: &str, n: usize) {
        let resources = self.resources(name);
        for _ in 0..n {
            resources.script().delete.push_ok(DeleteOutcome::Deleted);
        }
    }

    /// The writes `name`'s resource port received.
    pub fn writes(&self, name: &str) -> Vec<ResourceCall> {
        self.resources(name).mutating_calls()
    }

    pub fn dispatch(&self, cmd: Command, ctx: DispatchContext) -> Result<Outcome, DispatchError> {
        self.bus
            .dispatch(cmd, ctx)
            .now_or_never()
            .expect("dispatch does not wait on fakes")
    }

    /// Dispatches `cmd`, expecting a confirmation request.
    pub fn ask(&self, cmd: Command, ctx: DispatchContext) -> ConfirmationRequest {
        match self.dispatch(cmd, ctx) {
            Ok(Outcome::NeedsConfirmation(request)) => request,
            other => panic!("expected a confirmation request, got {other:?}"),
        }
    }

    /// Dispatches `cmd`, confirms it as the request asks, and returns the second result.
    pub fn confirm_and_run(
        &self,
        cmd: Command,
        ctx: DispatchContext,
    ) -> Result<Outcome, DispatchError> {
        let request = self.ask(cmd.clone(), ctx.clone());
        let answer = match request.expected_name {
            Some(name) => Confirmation::typed(request.token, name),
            None => Confirmation::simple(request.token),
        };
        self.dispatch(cmd, ctx.with_confirmation(answer))
    }

    pub fn audit(&self) -> Vec<AuditRecord> {
        self.state.audit_log()
    }

    pub fn outcomes(&self) -> Vec<(AuditOutcome, Initiator)> {
        self.audit()
            .iter()
            .map(|r| (r.outcome, r.initiator))
            .collect()
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().clone()
    }
}
