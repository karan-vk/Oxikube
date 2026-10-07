//! Node shells against the testkit fakes: a real bus and guard over fake sessions, so the
//! command goes through the read-only check, the confirmation and the audit like in the app, and
//! the pod is "created" on a `FakeExecPort` and dry-run on a `FakeResourcePort`.

mod audit;
mod guard;
mod open;

use std::sync::Arc;

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::{ClusterPrefs, NodeShellPrefs};
use oxikube_testkit::{FakeExecPort, FakeResourcePort};
use parking_lot::Mutex;

use super::register_command;
use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::exec::ExecService;
use crate::testing::{Harness, ctx, id, node};

/// A bus with the real `node::Shell` handler over cluster `a`.
pub(super) struct Fixture {
    pub h: Harness,
    pub service: Arc<ExecService>,
    pub exec: Arc<FakeExecPort>,
    pub resources: Arc<FakeResourcePort>,
    /// The nodes the handler asked the UI to open a terminal for.
    pub opened: Arc<Mutex<Vec<ResourceRef>>>,
}

impl Fixture {
    /// Cluster `a` with `prefs`.
    pub fn with(prefs: ClusterPrefs) -> Self {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let mut service = None;
        let h = Harness::with_extra(|registry, _, manager| {
            let built = Arc::new(ExecService::new(manager.clone()));
            let queue = opened.clone();
            let open: super::NodeShellOpener = Arc::new(move |node| {
                queue.lock().push(node.clone());
                Ok(())
            });
            service = Some(built.clone());
            register_command(registry, built, open)
        });
        let service = service.expect("the extra closure ran");
        service.set_audit(h.bus.guard().audit_handle());
        let read_only = prefs.read_only;
        h.prefs.seed(&id("a"), prefs);
        h.connect_configured("a");
        assert_eq!(h.manager.get(&id("a")).unwrap().read_only(), read_only);
        let ports = h.connector.ports_for(&id("a"));
        Self {
            service,
            exec: ports.exec,
            resources: ports.resources,
            opened,
            h,
        }
    }

    pub fn new() -> Self {
        Self::with(ClusterPrefs::default())
    }

    /// `worker-1` of cluster `a`.
    pub fn node(&self) -> ResourceRef {
        node("a", "worker-1")
    }

    pub fn command(&self) -> Command {
        Command::NodeShell {
            target: self.node(),
        }
    }

    /// Dispatches `node::Shell`, asks, confirms and runs it as the UI.
    pub fn run(&self) -> Result<Outcome, DispatchError> {
        self.h.confirm_and_run(self.command(), ctx(Initiator::Ui))
    }

    /// The same with another dispatch context (a dry run).
    pub fn run_with(&self, context: DispatchContext) -> Result<Outcome, DispatchError> {
        self.h.confirm_and_run(self.command(), context)
    }
}

/// Prefs with a custom image and template.
pub(super) fn custom_prefs() -> ClusterPrefs {
    ClusterPrefs {
        node_shell_image: Some("registry.local/tools:2".into()),
        node_shell_pull_secret: Some("regcred".into()),
        node_shell: NodeShellPrefs {
            namespace: Some("ops-debug".into()),
            nsenter_args: vec!["-t".into(), "1".into(), "-m".into(), "-n".into()],
            labels: [("team".to_owned(), "infra".to_owned())].into(),
            max_lifetime_seconds: Some(900),
            ..NodeShellPrefs::default()
        },
        ..ClusterPrefs::default()
    }
}
