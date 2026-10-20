//! The port-forward rig of the view tests (E15-S04): a real [`PortForwardManager`] over a backend
//! that serves nothing (only its read-only policy is read), and fake `portforward::Start` and
//! `pod::PortForward` handlers on the bus that record the command and answer as scripted. The
//! start dialog, the Forward buttons and `shift-f` are tested against it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext};
use oxikube_app::portforward::ObjectWatch;
use oxikube_app::store::{Spawner, StoreQuery};
use oxikube_app::{ForwardBackend, PortForwardManager, StartError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, PortForwardPort};
use parking_lot::Mutex;
use serde_json::json;

/// How the fake `portforward::Start` answers.
#[derive(Clone)]
pub(crate) enum Reply {
    /// The forward is up on `127.0.0.1:34567`, served by `web-0`.
    Up,
    /// The start fails with this error (typed, like the real handler's).
    Fail(StartError),
}

/// What the tests script and read.
#[derive(Clone)]
pub(crate) struct ForwardRig {
    /// Every forward command the bus ran, in order.
    pub calls: Arc<Mutex<Vec<Command>>>,
    /// The next answer.
    pub reply: Arc<Mutex<Reply>>,
    /// Whether the cluster is read-only.
    pub read_only: Arc<AtomicBool>,
    /// The manager the starter reads its policy from.
    pub manager: PortForwardManager,
}

struct Backend(Arc<AtomicBool>);

impl ForwardBackend for Backend {
    fn port_forward(&self, _: &ClusterId) -> Option<Arc<dyn PortForwardPort>> {
        None
    }

    fn is_read_only(&self, _: &ClusterId) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    fn watch(&self, _: &ClusterId, _: StoreQuery) -> Option<ObjectWatch> {
        None
    }
}

impl ForwardRig {
    pub(crate) fn new(clock: Arc<dyn ClockPort>) -> Self {
        let read_only = Arc::new(AtomicBool::new(false));
        let spawner: Arc<dyn Spawner> = Arc::new(|_: futures::future::BoxFuture<'static, ()>| {});
        Self {
            calls: Arc::default(),
            reply: Arc::new(Mutex::new(Reply::Up)),
            manager: PortForwardManager::new(
                Arc::new(Backend(read_only.clone())),
                spawner,
                clock,
                None,
            ),
            read_only,
        }
    }

    /// Installs the fake handlers.
    pub(crate) fn register(&self, registry: &mut CommandRegistry) {
        for id in [CommandId::PORTFORWARD_START, CommandId::POD_PORT_FORWARD] {
            let (calls, reply) = (self.calls.clone(), self.reply.clone());
            registry
                .register(
                    *command::lookup(id).unwrap(),
                    move |command: Command, _: HandlerContext| {
                        let (calls, reply) = (calls.clone(), reply.clone());
                        async move { answer(&calls, &reply, command) }
                    },
                )
                .unwrap();
        }
    }

    /// The `portforward::Start` commands that ran.
    pub(crate) fn starts(&self) -> Vec<Command> {
        self.calls
            .lock()
            .iter()
            .filter(|c| matches!(c, Command::PortForwardStart { .. }))
            .cloned()
            .collect()
    }
}

fn answer(
    calls: &Mutex<Vec<Command>>,
    reply: &Mutex<Reply>,
    command: Command,
) -> OxiResult<CommandOutput> {
    calls.lock().push(command);
    match reply.lock().clone() {
        Reply::Up => Ok(CommandOutput {
            message: Some("Forwarding 127.0.0.1:34567 to pod web-0".into()),
            data: Some(json!({"id": 7, "local_addr": "127.0.0.1:34567", "pod": "web-0"})),
        }),
        Reply::Fail(error) => Err(OxiError::from(error)),
    }
}
