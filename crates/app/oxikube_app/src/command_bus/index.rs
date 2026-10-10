//! [`CommandInfo`] and [`CommandIndex`]: the registered commands, sorted once, for the palette,
//! the help overlay and the keymap to list.

use std::collections::HashMap;
use std::sync::Arc;

use oxikube_domain::command::{Availability, CommandCategory, CommandId, CommandMeta};
use oxikube_domain::safety::ConfirmTier;

use super::availability::{CommandContext, Unavailable, check};

/// One registered command as the palette sees it: the declared metadata plus who registered
/// it and whether it has an MCP tool stub. Two words wide and `Copy`, so a list of 2 000 is
/// cheap to build and to hand to a picker.
#[derive(Debug, Clone, Copy)]
pub struct CommandInfo {
    meta: &'static CommandMeta,
    owner: &'static str,
    has_tool: bool,
}

impl CommandInfo {
    /// Describes a registered command. `meta` is the declared metadata
    /// (`oxikube_domain::command::lookup`), which registration checks the handler's equals.
    pub const fn new(meta: &'static CommandMeta, owner: &'static str, has_tool: bool) -> Self {
        Self {
            meta,
            owner,
            has_tool,
        }
    }

    /// The command's metadata.
    pub const fn meta(&self) -> &'static CommandMeta {
        self.meta
    }

    /// The command's stable id (`pod::Delete`).
    pub const fn id(&self) -> CommandId {
        self.meta.id
    }

    /// The palette title.
    pub const fn title(&self) -> &'static str {
        self.meta.title
    }

    /// The group the palette lists it under.
    pub const fn category(&self) -> CommandCategory {
        self.meta.category
    }

    /// The name keymap files bind it by.
    pub const fn keymap_action(&self) -> &'static str {
        self.meta.keymap_action()
    }

    /// Where and when it can run.
    pub const fn availability(&self) -> &'static Availability {
        &self.meta.availability
    }

    /// Whether it changes cluster state (the palette marks it).
    pub const fn mutating(&self) -> bool {
        self.meta.mutating
    }

    /// The confirmation the guard asks by default.
    pub const fn confirm(&self) -> ConfirmTier {
        self.meta.confirm
    }

    /// The crate that registered its handler.
    pub const fn owner(&self) -> &'static str {
        self.owner
    }

    /// Whether it has an MCP tool stub (privileged commands have none).
    pub const fn has_tool(&self) -> bool {
        self.has_tool
    }

    /// Whether the command can run in `ctx`.
    ///
    /// # Errors
    ///
    /// The first requirement `ctx` does not meet.
    pub fn check(&self, ctx: &CommandContext) -> Result<(), Unavailable> {
        check(self.meta, ctx)
    }
}

/// Two registered commands share an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("command {0} is listed twice")]
pub struct DuplicateCommand(pub CommandId);

/// The registered commands in display order (category, then title, then id), with lookup by id.
///
/// Built once when the bus is built and immutable after, so listing takes no lock and
/// [`list`](Self::list) is one pass over a presorted slice. Cheap to clone.
#[derive(Debug, Clone)]
pub struct CommandIndex {
    sorted: Arc<[CommandInfo]>,
    by_id: Arc<HashMap<CommandId, usize>>,
}

impl CommandIndex {
    /// Sorts `commands` by category, then title, then id (a total order, so the result is
    /// deterministic whatever the registration order).
    ///
    /// # Errors
    ///
    /// [`DuplicateCommand`] when two entries share an id.
    pub fn new(commands: impl IntoIterator<Item = CommandInfo>) -> Result<Self, DuplicateCommand> {
        let mut sorted: Vec<CommandInfo> = commands.into_iter().collect();
        sorted.sort_by_key(|info| (info.category(), info.title(), info.id()));
        let mut by_id = HashMap::with_capacity(sorted.len());
        for (index, info) in sorted.iter().enumerate() {
            if by_id.insert(info.id(), index).is_some() {
                return Err(DuplicateCommand(info.id()));
            }
        }
        Ok(Self {
            sorted: sorted.into(),
            by_id: Arc::new(by_id),
        })
    }

    /// Every command, in display order (the palette's "show all" and the help overlay).
    pub fn all(&self) -> &[CommandInfo] {
        &self.sorted
    }

    /// The command `id`, if registered.
    pub fn get(&self, id: CommandId) -> Option<CommandInfo> {
        self.by_id.get(&id).map(|&index| self.sorted[index])
    }

    /// The commands that can run in `ctx`, in display order. One pass over the index with no
    /// allocation besides the result.
    pub fn list(&self, ctx: &CommandContext) -> Vec<CommandInfo> {
        self.sorted
            .iter()
            .filter(|info| info.check(ctx).is_ok())
            .copied()
            .collect()
    }

    /// How many commands are registered.
    pub fn len(&self) -> usize {
        self.sorted.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.sorted.is_empty()
    }
}
