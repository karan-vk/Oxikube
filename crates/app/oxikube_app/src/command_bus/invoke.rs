//! [`CommandTarget`] and [`commands_for`]: from a command *id* and what the user has in front of
//! them to the [`Command`] values the bus dispatches.
//!
//! The command palette, the help overlay and the `:` jump bar list commands by id
//! ([`CommandInfo`](super::CommandInfo)); the bus dispatches `Command` payloads that carry their
//! operands (`pod::Delete { target }`, `resource::OpenList { cluster, gvk }`). This is the one
//! place that turns the first into the second, generically, so a surface never matches on
//! command ids and a new command needs no change here:
//!
//! - the operands come from the [`CommandTarget`] the surface captured from the focused view (the
//!   active cluster, the kind of the table, the selected objects);
//! - a command that acts on objects yields one `Command` per selected object (as the table's own
//!   row actions do); a command that acts on the kind or the cluster yields one;
//! - a command that needs an operand the target cannot supply (a replica count, a label key, a
//!   port) is [`InvokeError::NeedsInput`]: its own dialog collects it, the palette stays generic.
//!
//! The payloads are built through their serde form (`{"type": "<id>", ...operands}`), the same
//! shape a keymap entry or an MCP tool call has, so "palette entry", "key" and "tool call" cannot
//! drift apart. Plain Rust: no UI, no I/O.

use std::collections::HashSet;

use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use serde_json::{Map, Value, json};

/// What a command acts on, as the surface that lists commands knows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandTarget {
    /// The active cluster.
    pub cluster: Option<ClusterId>,
    /// The kind the focused view lists (a resource table's kind).
    pub gvk: Option<Gvk>,
    /// The objects the focused view acts on: its selection, else its cursor row.
    pub targets: Vec<ResourceRef>,
}

impl CommandTarget {
    /// No cluster, no kind, no objects: the catalog or an empty workspace.
    pub fn none() -> Self {
        Self::default()
    }

    /// The same target in `cluster`.
    #[must_use]
    pub fn in_cluster(mut self, cluster: ClusterId) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// The same target listing `gvk`.
    #[must_use]
    pub fn of_kind(mut self, gvk: Gvk) -> Self {
        self.gvk = Some(gvk);
        self
    }

    /// The same target acting on `targets`.
    #[must_use]
    pub fn selecting(mut self, targets: Vec<ResourceRef>) -> Self {
        self.targets = targets;
        self
    }
}

/// Why a command could not be built from a [`CommandTarget`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvokeError {
    /// The command needs an operand the target cannot supply.
    #[error("{id} needs more information than the palette can supply ({detail})")]
    NeedsInput {
        /// The command.
        id: CommandId,
        /// The first missing operand, as serde names it.
        detail: String,
    },
    /// No command is declared under the id.
    #[error("{0} is not a declared command")]
    Unknown(CommandId),
}

/// The commands to dispatch to run `id` on `target`: one per selected object when it acts on
/// objects, else one. See the [module docs](self).
///
/// # Errors
///
/// [`InvokeError::NeedsInput`] when `target` lacks an operand, [`InvokeError::Unknown`] for an
/// undeclared id.
pub fn commands_for(id: CommandId, target: &CommandTarget) -> Result<Vec<Command>, InvokeError> {
    if oxikube_domain::command::lookup(id).is_none() {
        return Err(InvokeError::Unknown(id));
    }
    let mut commands: Vec<Command> = Vec::new();
    if target.targets.is_empty() {
        commands.push(build(id, target, None)?);
    } else {
        // A command that does not act on objects (`resource::OpenList`) comes out the same for
        // every selected object: it runs once. Seen commands are kept by their serde form, so the
        // dedup is linear in the selection (a select-all of 10k pods), not quadratic.
        let mut seen: HashSet<String> = HashSet::with_capacity(target.targets.len());
        for object in &target.targets {
            let command = build(id, target, Some(object))?;
            let fresh = match serde_json::to_string(&command) {
                Ok(key) => seen.insert(key),
                Err(_) => !commands.contains(&command),
            };
            if fresh {
                commands.push(command);
            }
        }
    }
    Ok(commands)
}

fn build(
    id: CommandId,
    target: &CommandTarget,
    object: Option<&ResourceRef>,
) -> Result<Command, InvokeError> {
    let mut fields = Map::new();
    fields.insert("type".into(), json!(id.as_str()));
    let cluster = object.map(|o| &o.cluster).or(target.cluster.as_ref());
    if let Some(cluster) = cluster {
        fields.insert("cluster".into(), json!(cluster));
    }
    let gvk = object.map(|o| &o.gvk).or(target.gvk.as_ref());
    if let Some(gvk) = gvk {
        fields.insert("gvk".into(), json!(gvk));
    }
    if let Some(object) = object {
        fields.insert("target".into(), json!(object));
    }
    serde_json::from_value::<Command>(Value::Object(fields)).map_err(|error| {
        InvokeError::NeedsInput {
            id,
            detail: error.to_string(),
        }
    })
}
