//! Execution: a plan's commands go through a real `CommandBus`, and the handlers see exactly the
//! navigation commands (a recording bus stands in for the UI's handlers).

use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::Gvk;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use parking_lot::Mutex;

use super::super::{JumpPlan, plan};
use super::{Env, cluster};
use crate::command_bus::{
    CommandBus, CommandOutput, CommandRegistry, DispatchContext, HandlerContext, Outcome,
};
use crate::guard::MutationGuard;
use crate::session::ClusterSessionManager;

/// Every command the jump bar can send.
const NAVIGATION: [CommandId; 10] = [
    CommandId::APP_QUIT,
    CommandId::CLUSTER_CONNECT,
    CommandId::CLUSTER_SELECT,
    CommandId::JUMP_BACK,
    CommandId::JUMP_FORWARD,
    CommandId::JUMP_LAST,
    CommandId::NAMESPACE_SELECT,
    CommandId::RESOURCE_OPEN_LIST,
    CommandId::TABLE_SET_FILTER,
    CommandId::VIEW_OPEN,
];

/// A bus whose handlers record the command they got.
struct RecordingBus {
    bus: CommandBus,
    seen: Arc<Mutex<Vec<Command>>>,
}

impl RecordingBus {
    fn new() -> Self {
        let seen: Arc<Mutex<Vec<Command>>> = Arc::default();
        let mut registry = CommandRegistry::new();
        for id in NAVIGATION {
            let seen = seen.clone();
            let meta = *command::lookup(id).expect("declared");
            registry
                .register(meta, move |command: Command, _: HandlerContext| {
                    seen.lock().push(command);
                    async { Ok(CommandOutput::none()) }
                })
                .expect("registers once");
        }
        let clock = Arc::new(FakeClockPort::default());
        let manager = ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            Arc::new(FakeClusterSourcePort::new()),
            clock.clone(),
        );
        let guard = MutationGuard::new(manager, Arc::new(FakeStatePort::new()), clock);
        Self {
            bus: CommandBus::new(registry, guard),
            seen,
        }
    }

    /// Sends the plan's commands in order, as the bar does, and returns what the handlers saw.
    fn run(&self, plan: &JumpPlan) -> Vec<Command> {
        for command in &plan.commands {
            let context = DispatchContext::new(Initiator::Ui, "tester");
            let outcome = self
                .bus
                .dispatch(command.clone(), context)
                .now_or_never()
                .expect("the handlers answer at once")
                .unwrap_or_else(|e| panic!("{}: {e}", command.id()));
            assert!(matches!(outcome, Outcome::Completed(_)), "{}", command.id());
        }
        std::mem::take(&mut *self.seen.lock())
    }
}

#[test]
fn deploy_kube_system_dispatches_the_namespace_and_the_list() {
    let bus = RecordingBus::new();
    let seen = bus.run(&plan("deploy kube-system", &Env::new()).unwrap());
    assert_eq!(
        seen,
        [
            Command::NamespaceSelect {
                cluster: cluster("dev"),
                namespaces: vec!["kube-system".to_owned()],
            },
            Command::ResourceOpenList {
                cluster: cluster("dev"),
                gvk: Gvk::new("apps", "v1", "Deployment"),
            },
        ]
    );
}

#[test]
fn pod_app_nginx_dispatches_the_list_and_the_selector() {
    let bus = RecordingBus::new();
    let seen = bus.run(&plan("pod app=nginx", &Env::new()).unwrap());
    assert_eq!(
        seen,
        [
            Command::ResourceOpenList {
                cluster: cluster("dev"),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::TableSetFilter {
                cluster: cluster("dev"),
                gvk: Gvk::new("", "v1", "Pod"),
                text: "-l app=nginx".to_owned(),
            },
        ]
    );
}

#[test]
fn ctx_prod_dispatches_the_tab_switch() {
    let bus = RecordingBus::new();
    let seen = bus.run(&plan("ctx prod-eu", &Env::new()).unwrap());
    assert_eq!(
        seen,
        [Command::ClusterSelect {
            cluster: cluster("prod-eu")
        }]
    );
}

#[test]
fn a_crd_alias_dispatches_the_crds_list() {
    let bus = RecordingBus::new();
    let seen = bus.run(&plan("certs", &Env::new()).unwrap());
    assert_eq!(
        seen,
        [Command::ResourceOpenList {
            cluster: cluster("dev"),
            gvk: Gvk::new("cert-manager.io", "v1", "Certificate"),
        }]
    );
}

#[test]
fn q_dispatches_the_quit_command() {
    let bus = RecordingBus::new();
    assert_eq!(
        bus.run(&plan("q", &Env::new()).unwrap()),
        [Command::AppQuit]
    );
}

#[test]
fn every_planned_command_is_a_declared_read_with_a_tool_stub() {
    // Parsed input executes only through Commands, and none of the navigation changes a cluster.
    let bus = RecordingBus::new();
    for line in [
        "pods",
        "deploy web /api app=x @prod-eu",
        "ns",
        "ns web",
        "ctx",
        "ctx staging",
        "q",
        "-",
        "[",
        "]",
    ] {
        let plan = plan(line, &Env::new()).unwrap();
        let after = plan
            .after_connect
            .iter()
            .flat_map(|a| a.commands.iter())
            .cloned();
        for command in plan.commands.iter().cloned().chain(after) {
            assert!(!command.is_mutating(), "{line}: {}", command.id());
            assert!(
                bus.bus
                    .tools()
                    .any(|t| t.name.to_string() == command.id().tool_name()),
                "{line}: {} has no tool stub",
                command.id()
            );
        }
    }
}
