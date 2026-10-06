//! [`CommandRegistry`]: where each crate's `init` registers its command handlers.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use oxikube_domain::OxiError;
use oxikube_domain::command::{self, CommandId, CommandMeta};
use oxikube_ports::ToolDef;
use serde_json::json;

use super::handler::CommandHandler;

/// The owner recorded for handlers registered outside [`CommandRegistry::install`].
const UNNAMED_OWNER: &str = "(unnamed)";

/// Why a registration was rejected. Startup code treats every one as a bug
/// (`.expect(..)` in the crate's `init`), so a duplicate or stale registration fails the
/// first test run rather than shipping.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// Two handlers for one id.
    #[error("{id} is registered twice: by {first} and by {second}")]
    Duplicate {
        /// The id.
        id: CommandId,
        /// The crate that registered it first.
        first: &'static str,
        /// The crate that tried again.
        second: &'static str,
    },
    /// The id is not declared in `oxikube_domain::command::COMMANDS`, so no
    /// [`Command`](oxikube_domain::command::Command) can carry it.
    #[error("{0} is not a declared command (add it to oxikube_domain::command first)")]
    Undeclared(CommandId),
    /// The metadata differs from the declared metadata. The guard's decisions (mutating,
    /// risk, tier) come from the declaration only, so a handler cannot re-describe a
    /// command to weaken them.
    #[error("{0}: registered metadata differs from its declaration")]
    MetaMismatch(CommandId),
    /// The MCP tool stub could not be built.
    #[error("{id}: tool stub rejected: {source}")]
    ToolStub {
        /// The id.
        id: CommandId,
        /// Why.
        source: OxiError,
    },
}

/// One registered command.
pub(crate) struct Registered {
    pub(crate) meta: CommandMeta,
    pub(crate) handler: Arc<dyn CommandHandler>,
    pub(crate) tool: Option<ToolDef>,
    pub(crate) owner: &'static str,
}

/// Collects command handlers before the [`CommandBus`](super::CommandBus) is built.
///
/// # Per-crate registration
///
/// Each crate that owns commands exposes a registration function and `bins/oxikube`
/// installs them in its init order, under the crate's name:
///
/// ```ignore
/// // in oxikube_catalog_ui
/// pub fn register_commands(registry: &mut CommandRegistry) -> Result<(), RegisterError> {
///     registry.register(*command::lookup(CommandId::CLUSTER_SELECT).unwrap(), select_cluster)
/// }
/// // in bins/oxikube
/// registry.install("oxikube_catalog_ui", oxikube_catalog_ui::register_commands).expect("commands");
/// ```
///
/// Feature crates never edit this crate to add a command. Every registration also
/// registers the command's MCP tool stub ([`ToolDef::for_command`]), except for
/// privileged commands, which ADR 0012 keeps out of agents' reach.
pub struct CommandRegistry {
    pub(crate) entries: IndexMap<CommandId, Registered>,
    owner: &'static str,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CommandRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.entries.keys()).finish()
    }
}

impl CommandRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            entries: IndexMap::new(),
            owner: UNNAMED_OWNER,
        }
    }

    /// Runs one crate's registration function, recording `owner` (the crate name) on
    /// everything it registers so a duplicate names both crates.
    ///
    /// # Errors
    ///
    /// The first [`RegisterError`] `init` returned.
    pub fn install<F>(&mut self, owner: &'static str, init: F) -> Result<(), RegisterError>
    where
        F: FnOnce(&mut CommandRegistry) -> Result<(), RegisterError>,
    {
        let previous = std::mem::replace(&mut self.owner, owner);
        let result = init(self);
        self.owner = previous;
        result
    }

    /// Registers `handler` for the command `meta` describes, plus its MCP tool stub.
    ///
    /// `meta` must be the declared metadata (`oxikube_domain::command::lookup(id)`).
    ///
    /// # Errors
    ///
    /// [`RegisterError::Duplicate`], [`Undeclared`](RegisterError::Undeclared),
    /// [`MetaMismatch`](RegisterError::MetaMismatch) or
    /// [`ToolStub`](RegisterError::ToolStub); nothing is registered then.
    pub fn register(
        &mut self,
        meta: CommandMeta,
        handler: impl CommandHandler + 'static,
    ) -> Result<(), RegisterError> {
        let id = meta.id;
        if let Some(first) = self.entries.get(&id) {
            return Err(RegisterError::Duplicate {
                id,
                first: first.owner,
                second: self.owner,
            });
        }
        let declared = command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
        if *declared != meta {
            return Err(RegisterError::MetaMismatch(id));
        }
        let tool = tool_stub(&meta)?;
        self.entries.insert(
            id,
            Registered {
                meta,
                handler: Arc::new(handler),
                tool,
                owner: self.owner,
            },
        );
        Ok(())
    }

    /// Whether `id` has a handler.
    pub fn contains(&self, id: CommandId) -> bool {
        self.entries.contains_key(&id)
    }

    /// How many commands are registered.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The MCP tool stub for `meta`: `None` for privileged commands.
///
/// The input schema is a permissive object until the agent phase (E-agent) generates
/// the real one from the `Command` payload; the name, title, risk and capabilities are
/// final.
fn tool_stub(meta: &CommandMeta) -> Result<Option<ToolDef>, RegisterError> {
    if meta.privileged {
        return Ok(None);
    }
    let description = format!(
        "{} (stub). Arguments are the fields of the `{}` command payload.",
        meta.title, meta.id
    );
    let schema = json!({ "type": "object", "additionalProperties": true });
    let tool = ToolDef::for_command(meta, description, schema)
        .and_then(|tool| tool.validate().map(|()| tool))
        .map_err(|source| RegisterError::ToolStub {
            id: meta.id,
            source,
        })?;
    Ok(Some(tool))
}
