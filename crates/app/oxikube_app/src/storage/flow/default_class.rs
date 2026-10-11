//! [`DefaultClassFlow`]: the "set default" dialog's reads and send.

use std::sync::Arc;

use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::storage::StorageClassSummary;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::ListOptions;

use super::confirm::dispatch_confirmed;
use crate::storage::commands::class_gvk;
use crate::{ClusterSessionManager, CommandBus};

/// What the dialog shows, read when it opens.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultClassContext {
    /// The class to make the default.
    pub target: ResourceRef,
    /// The class as it is now.
    pub class: StorageClassSummary,
    /// The other classes that are the default now (a cluster should have one): the dialog
    /// offers to clear them.
    pub previous: Vec<Arc<str>>,
    /// The cluster's context name, for the dialog's text.
    pub context: String,
}

impl DefaultClassContext {
    /// Whether the class is the default already and nothing else is (there is nothing to do).
    pub fn is_noop(&self) -> bool {
        self.class.is_default && self.previous.is_empty()
    }
}

/// What the command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultClassReport {
    /// The handler's one-line message.
    pub message: String,
    /// The classes that stopped being the default.
    pub cleared: Vec<String>,
}

/// Opens and runs "set default StorageClass". Cheap to clone.
#[derive(Clone)]
pub struct DefaultClassFlow {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    who: Arc<str>,
}

impl std::fmt::Debug for DefaultClassFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultClassFlow").finish_non_exhaustive()
    }
}

impl DefaultClassFlow {
    /// A flow dispatching on `bus` as `who`, reading through `sessions`.
    pub fn new(bus: CommandBus, sessions: ClusterSessionManager, who: impl Into<Arc<str>>) -> Self {
        Self {
            bus,
            sessions,
            who: who.into(),
        }
    }

    /// Reads the classes. Plain async: spawn it on the Tokio bridge.
    ///
    /// # Errors
    ///
    /// The read's error, or `NotFound` when the class is gone.
    pub async fn open(&self, target: &ResourceRef) -> OxiResult<DefaultClassContext> {
        let session = self
            .sessions
            .get(&target.cluster)
            .ok_or_else(|| OxiError::not_found("the cluster is not open"))?;
        let reader = session
            .resources()
            .ok_or_else(|| OxiError::not_found("the cluster is not connected"))?;
        let page = reader
            .list(&class_gvk(), None, &ListOptions::default())
            .await?;
        let classes: Vec<StorageClassSummary> = page
            .items
            .iter()
            .filter_map(|res| StorageClassSummary::from_resource(res).ok())
            .collect();
        let class = classes
            .iter()
            .find(|c| *c.name == *target.name)
            .cloned()
            .ok_or_else(|| {
                OxiError::not_found(format!("StorageClass {} not found", target.name))
            })?;
        let previous = classes
            .iter()
            .filter(|c| c.is_default && c.name != class.name)
            .map(|c| c.name.clone())
            .collect();
        Ok(DefaultClassContext {
            target: target.clone(),
            class,
            previous,
            context: session.context().to_string(),
        })
    }

    /// Makes the class the default through the bus: `storageclass::SetDefault`, with the
    /// guard's simple confirmation answered for the dialog's confirm button.
    ///
    /// # Errors
    ///
    /// The refusal or failure as an [`OxiError`] (read-only is `Forbidden`).
    pub async fn run(
        &self,
        context: &DefaultClassContext,
        clear_previous: bool,
    ) -> OxiResult<DefaultClassReport> {
        let command = Command::StorageClassSetDefault {
            target: context.target.clone(),
            clear_previous,
        };
        let output = dispatch_confirmed(&self.bus, &self.who, &context.target.cluster, command)
            .await
            .map_err(OxiError::from)?;
        let cleared = output
            .data
            .as_ref()
            .and_then(|d| d.get("cleared"))
            .and_then(|c| c.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Ok(DefaultClassReport {
            message: output.message.unwrap_or_default(),
            cleared,
        })
    }
}
