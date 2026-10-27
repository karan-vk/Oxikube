//! [`DrainFlow`]: what the drain dialog reads before it asks, and how it starts and follows the
//! drain.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::drain::{DrainFlags, DrainPlan, DrainProgress, plan_drain};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, ResourceReader};

use super::service::NodeService;
use crate::CommandBus;
use crate::command_bus::{DispatchContext, Outcome};
use crate::guard::Confirmation;

/// Pods are listed this many at a time.
const PAGE: u32 = 500;

/// The node and its pods, read when the dialog opens: what the plan is made from.
#[derive(Debug, Clone, PartialEq)]
pub struct DrainContext {
    /// The node.
    pub target: ResourceRef,
    /// The cluster's context name, for the dialog's text.
    pub context: String,
    /// Whether the node is unschedulable already.
    pub cordoned: bool,
    /// Every pod bound to the node (finished ones too: a drain evicts them), by namespace and
    /// name.
    pub pods: Vec<Resource>,
}

impl DrainContext {
    /// What a drain with `flags` would evict, skip and refuse, by kubectl's rules (the same
    /// function the adapter applies, again, once the node is cordoned).
    pub fn plan(&self, flags: &DrainFlags) -> DrainPlan {
        plan_drain(&self.pods, &flags.options())
    }
}

/// A drain that was asked for: its steps as they happen and its end.
pub struct DrainStart {
    /// Every step of the drain, in order; ends when the drain ends.
    pub progress: mpsc::UnboundedReceiver<DrainProgress>,
    /// The dispatch: the handler's message when the drain finished with every pod gone, else the
    /// refusal or failure. Run it on the Tokio bridge; **dropping it cancels the drain** (no
    /// further eviction; the node stays cordoned; the guard audits it as cancelled).
    pub run: BoxFuture<'static, OxiResult<String>>,
}

/// Opens, starts and follows drains for the dialog. Cheap to clone.
#[derive(Clone)]
pub struct DrainFlow {
    bus: CommandBus,
    service: Arc<NodeService>,
    who: Arc<str>,
}

impl std::fmt::Debug for DrainFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrainFlow").finish_non_exhaustive()
    }
}

impl DrainFlow {
    /// A flow dispatching on `bus` as `who` (the local user's name, for the audit log), following
    /// drains through `service`'s monitor.
    pub fn new(bus: CommandBus, service: Arc<NodeService>, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            service,
            who: who.into(),
        }
    }

    /// Reads the node and every pod on it (`spec.nodeName=<node>`, all namespaces, every page).
    /// Plain async: spawn it on the Tokio bridge.
    ///
    /// # Errors
    ///
    /// The read's error (forbidden, not found, not connected).
    pub async fn open(&self, target: &ResourceRef) -> OxiResult<DrainContext> {
        let session = self
            .service
            .sessions()
            .get(&target.cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        let reader = session
            .resources()
            .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
        let node = reader
            .get(&Gvk::new("", "v1", "Node"), None, &target.name)
            .await?;
        let pods = pods_on(&*reader, &target.name).await?;
        Ok(DrainContext {
            target: target.clone(),
            context: session.context().to_string(),
            cordoned: node.get_bool("/spec/unschedulable") == Some(true),
            pods,
        })
    }

    /// Whether a drain of `target` is running (from this window or another caller).
    pub fn is_running(&self, target: &ResourceRef) -> bool {
        self.service
            .monitor()
            .is_running(&target.cluster, &target.name)
    }

    /// Starts draining `target` with `flags`: follows it first, then dispatches `node::Drain` as
    /// the user, answering the guard's type-the-name confirmation with `typed` (what the user
    /// typed; the guard checks it against the node's name).
    pub fn start(&self, target: &ResourceRef, flags: DrainFlags, typed: String) -> DrainStart {
        let progress = self
            .service
            .monitor()
            .subscribe(&target.cluster, &target.name);
        let bus = self.bus.clone();
        let who = self.who.clone();
        let target = target.clone();
        let run = async move {
            let cluster = target.cluster.clone();
            let command = Command::NodeDrain { target, flags };
            let context =
                || DispatchContext::new(Initiator::Ui, who.clone()).with_cluster(cluster.clone());
            let outcome = match bus.dispatch(command.clone(), context()).await? {
                Outcome::NeedsConfirmation(request) => {
                    let answer = match request.tier {
                        ConfirmTier::TypeName => Confirmation::typed(request.token, typed),
                        _ => Confirmation::simple(request.token),
                    };
                    bus.dispatch(command, context().with_confirmation(answer))
                        .await?
                }
                done @ Outcome::Completed(_) => done,
            };
            match outcome {
                Outcome::Completed(output) => Ok(output.message.unwrap_or_default()),
                Outcome::NeedsConfirmation(_) => {
                    Err(OxiError::conflict("the confirmation was not accepted"))
                }
            }
        };
        DrainStart {
            progress,
            run: run.boxed(),
        }
    }
}

/// Every pod bound to `node`, sorted by namespace and name.
async fn pods_on(reader: &dyn ResourceReader, node: &str) -> OxiResult<Vec<Resource>> {
    let kind = Gvk::new("", "v1", "Pod");
    let mut options = ListOptions::default()
        .fields(format!("spec.nodeName={node}"))
        .limit(PAGE);
    let mut pods = Vec::new();
    loop {
        let page = reader.list(&kind, None, &options).await?;
        // The field selector does the work; this guards a server that ignores it.
        pods.extend(
            page.items
                .into_iter()
                .filter(|pod| pod.get_str("/spec/nodeName") == Some(node)),
        );
        match page.continue_token.filter(|token| !token.is_empty()) {
            Some(token) => options = options.continue_from(token),
            None => break,
        }
    }
    pods.sort_by(|a, b| (a.namespace(), a.name()).cmp(&(b.namespace(), b.name())));
    Ok(pods)
}
