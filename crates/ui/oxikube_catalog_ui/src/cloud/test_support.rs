//! Helpers for tests of the Cloud section and of the crates built on it (feature
//! `test-support`): a backend that records the commands it was sent and plays scripted events,
//! and sample clusters.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use gpui::{App, Task};
use oxikube_app::cloud::{CloudDiscoveryEvent, ManagedEntry};
use oxikube_app::command_bus::CommandOutput;
use oxikube_domain::OxiResult;
use oxikube_domain::cloud::{
    CloudProvider, CloudScope, ClusterStatus, DiscoveredCluster, ExecInteractiveMode, ExecPlugin,
    KubeconfigEntry,
};
use oxikube_domain::command::Command;
use oxikube_domain::ids::ContextName;

use super::backend::CloudBackend;

/// A [`CloudBackend`] that records every command and lets a test push discovery events. Cheap to
/// clone; clones share the record, the events and the added clusters.
#[derive(Clone, Default)]
pub struct ScriptedBackend {
    sent: Rc<RefCell<Vec<Command>>>,
    added: Rc<RefCell<Vec<ManagedEntry>>>,
    watchers: Rc<RefCell<Vec<mpsc::UnboundedSender<Arc<CloudDiscoveryEvent>>>>>,
    hold: Rc<Cell<bool>>,
}

impl ScriptedBackend {
    /// A backend with nothing added.
    pub fn new() -> Self {
        Self::default()
    }

    /// The commands sent so far, oldest first.
    pub fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    /// Plays `event` to the view.
    pub fn emit(&self, event: CloudDiscoveryEvent) {
        let event = Arc::new(event);
        self.watchers
            .borrow_mut()
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }

    /// While `hold` is set, commands are recorded but never finish: a discovery that is still
    /// running.
    pub fn hold(&self, hold: bool) {
        self.hold.set(hold);
    }

    /// Sets the clusters the managed kubeconfigs hold.
    pub fn set_added(&self, entries: Vec<ManagedEntry>) {
        *self.added.borrow_mut() = entries;
    }
}

impl CloudBackend for ScriptedBackend {
    fn run(&self, command: Command, cx: &mut App) -> Task<OxiResult<CommandOutput>> {
        self.sent.borrow_mut().push(command);
        if self.hold.get() {
            return cx.spawn(async |_| futures::future::pending().await);
        }
        Task::ready(Ok(CommandOutput::message("done")))
    }

    fn watch(&self) -> BoxStream<'static, Arc<CloudDiscoveryEvent>> {
        let (tx, rx) = mpsc::unbounded();
        self.watchers.borrow_mut().push(tx);
        rx.boxed()
    }

    fn added(&self, _: CloudProvider, _: &mut App) -> Task<OxiResult<Vec<ManagedEntry>>> {
        Task::ready(Ok(self.added.borrow().clone()))
    }
}

/// An AWS scope.
pub fn aws_scope(profile: &str, region: &str) -> CloudScope {
    CloudScope::Aws {
        profile: profile.into(),
        region: region.into(),
    }
}

/// A running EKS cluster in `scope`, version 1.30.
///
/// # Panics
///
/// When `scope` is not an AWS scope.
pub fn eks(scope: &CloudScope, name: &str) -> DiscoveredCluster {
    let CloudScope::Aws { region, .. } = scope else {
        panic!("an AWS scope");
    };
    DiscoveredCluster {
        kubernetes_version: Some("1.30".into()),
        status: ClusterStatus::Running,
        ..DiscoveredCluster::new(scope.clone(), name, region.clone())
    }
}

/// The managed-file entry of `cluster`, as `CloudDiscoveryService::added` returns it.
pub fn managed(cluster: &DiscoveredCluster) -> ManagedEntry {
    ManagedEntry {
        entry: KubeconfigEntry {
            context: ContextName::new(format!("eks-{}", cluster.name)),
            server: "https://example.invalid".into(),
            ca_data: None,
            exec: ExecPlugin::new("aws", Vec::new(), ExecInteractiveMode::IfAvailable),
        },
        cluster_key: Some(cluster.key()),
    }
}
