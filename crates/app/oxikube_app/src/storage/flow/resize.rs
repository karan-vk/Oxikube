//! [`ResizeFlow`]: the PVC resize dialog's reads, check and send.

use std::sync::Arc;

use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::Condition;
use oxikube_domain::view::storage::{
    Expansion, PvcSummary, ResizeBlock, StorageClassSummary, check_resize,
};
use oxikube_domain::{OxiError, OxiResult, Quantity};

use super::confirm::dispatch_confirmed;
use crate::storage::read::read_claim;
use crate::{ClusterSessionManager, CommandBus};

/// What the resize dialog shows, read from the cluster when it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct ResizeContext {
    /// The claim to resize.
    pub target: ResourceRef,
    /// The claim as it is now.
    pub claim: PvcSummary,
    /// Its StorageClass, when it names one that could be read.
    pub class: Option<StorageClassSummary>,
    /// Whether the class lets the claim grow.
    pub expansion: Expansion,
    /// The cluster's context name, for the dialog's text.
    pub context: String,
}

impl ResizeContext {
    /// The size the new one must exceed: what the claim asks for now.
    pub fn current(&self) -> Option<Quantity> {
        self.claim.current_request()
    }

    /// Why a resize cannot be offered at all (the class forbids it, or there is none), before
    /// anything is typed.
    pub fn blocked(&self) -> Option<ResizeBlock> {
        match &self.expansion {
            Expansion::NoClass => Some(ResizeBlock::NoStorageClass),
            Expansion::Disallowed { class } => Some(ResizeBlock::ExpansionDisallowed {
                class: class.clone(),
            }),
            Expansion::Allowed | Expansion::Unknown { .. } => None,
        }
    }

    /// Checks `input` as the new size: greater than the current one, and the class allows it.
    ///
    /// # Errors
    ///
    /// The [`ResizeBlock`] that refuses it; nothing is sent.
    pub fn check(&self, input: &str) -> Result<Quantity, ResizeBlock> {
        check_resize(self.current().as_ref(), input, &self.expansion)
    }
}

/// What a resize did.
#[derive(Debug, Clone, PartialEq)]
pub struct ResizeReport {
    /// The size before.
    pub from: Option<Quantity>,
    /// The size asked for.
    pub to: Quantity,
    /// The claim's conditions straight after the patch (a pending file-system expansion shows
    /// here), for the dialog to show.
    pub conditions: Vec<Condition>,
    /// The handler's one-line message.
    pub message: String,
}

/// Opens, checks and runs PVC resizes. Cheap to clone.
#[derive(Clone)]
pub struct ResizeFlow {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    who: Arc<str>,
}

impl std::fmt::Debug for ResizeFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResizeFlow").finish_non_exhaustive()
    }
}

impl ResizeFlow {
    /// A flow dispatching on `bus` as `who` (the local user's name, for the audit log), reading
    /// through `sessions`.
    pub fn new(bus: CommandBus, sessions: ClusterSessionManager, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            sessions,
            who: who.into(),
        }
    }

    /// Reads the claim and its class. Plain async: spawn it on the Tokio bridge.
    ///
    /// # Errors
    ///
    /// The read's error (forbidden, not found, not connected).
    pub async fn open(&self, target: &ResourceRef) -> OxiResult<ResizeContext> {
        let session = self
            .sessions
            .get(&target.cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        let reader = session
            .resources()
            .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
        let (claim, class) = read_claim(&*reader, target.namespace(), &target.name).await?;
        let expansion = Expansion::of(claim.storage_class.as_deref(), class.as_ref());
        Ok(ResizeContext {
            target: target.clone(),
            claim,
            class,
            expansion,
            context: session.context().to_string(),
        })
    }

    /// Resizes the claim to `size` through the bus: `pvc::Resize` with the size the dialog
    /// showed as `from`, which the handler checks against the live claim and the audit record
    /// carries. The guard's simple confirmation is answered for the dialog's confirm button.
    ///
    /// # Errors
    ///
    /// The refusal or failure as an [`OxiError`]: read-only (`Forbidden`), the server's refusal,
    /// or a claim that changed since the dialog opened (`Conflict`).
    pub async fn run(&self, context: &ResizeContext, size: Quantity) -> OxiResult<ResizeReport> {
        let from = context.current();
        let command = Command::PvcResize {
            target: context.target.clone(),
            size: size.to_string(),
            from: from.map(|q| q.to_string()),
        };
        let output = dispatch_confirmed(&self.bus, &self.who, &context.target.cluster, command)
            .await
            .map_err(OxiError::from)?;
        let conditions = output
            .data
            .as_ref()
            .and_then(|d| d.get("conditions"))
            .and_then(|c| c.as_array())
            .map(|list| list.iter().filter_map(condition).collect())
            .unwrap_or_default();
        Ok(ResizeReport {
            from,
            to: size,
            conditions,
            message: output.message.unwrap_or_default(),
        })
    }
}

/// A condition as the handler reported it.
fn condition(value: &serde_json::Value) -> Option<Condition> {
    use oxikube_domain::view::ConditionStatus;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str());
    Some(Condition {
        kind: Arc::from(text("type")?),
        status: ConditionStatus::parse(text("status")),
        reason: None,
        message: text("message").map(Arc::from),
        last_transition: None,
    })
}
