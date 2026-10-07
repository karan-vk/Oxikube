//! The one-shot permits that tie an opened node shell to a guarded `node::Shell`.

use std::time::{Duration, Instant};

use oxikube_domain::audit::Initiator;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::NodeShellSpec;
use parking_lot::Mutex;

/// How long a permit waits for its terminal to open. The tab opens in the same UI turn the
/// handler queued it, so this is generous; an unclaimed permit must not become a standing
/// authorisation.
const PERMIT_TTL: Duration = Duration::from_secs(120);

/// Most permits kept at once; the oldest is dropped past it.
const MAX_PERMITS: usize = 16;

/// What the guarded handler decided: this node, from this template, for this initiator.
#[derive(Debug, Clone)]
pub(super) struct Permit {
    pub(super) node: ResourceRef,
    pub(super) spec: NodeShellSpec,
    pub(super) who: String,
    pub(super) initiator: Initiator,
    issued: Instant,
}

/// The permits waiting for their terminal.
#[derive(Debug, Default)]
pub(in crate::exec) struct Permits {
    waiting: Mutex<Vec<Permit>>,
}

impl Permits {
    /// Leaves a permit for a shell on `node`.
    pub(super) fn grant(
        &self,
        node: ResourceRef,
        spec: NodeShellSpec,
        who: &str,
        initiator: Initiator,
    ) {
        let mut waiting = self.waiting.lock();
        waiting.retain(|permit| permit.issued.elapsed() < PERMIT_TTL);
        if waiting.len() >= MAX_PERMITS {
            waiting.remove(0);
        }
        waiting.push(Permit {
            node,
            spec,
            who: who.to_owned(),
            initiator,
            issued: Instant::now(),
        });
    }

    /// Takes the oldest live permit for `node`: each guarded command opens one terminal.
    pub(super) fn take(&self, node: &ResourceRef) -> Option<Permit> {
        let mut waiting = self.waiting.lock();
        waiting.retain(|permit| permit.issued.elapsed() < PERMIT_TTL);
        let at = waiting.iter().position(|permit| permit.node == *node)?;
        Some(waiting.remove(at))
    }
}
