//! The action registry: which action names exist, grouped by namespace, and which of them are
//! [`Command`](oxikube_domain::command::Command)s.
//!
//! GPUI already registers every `actions!` / `#[derive(Action)]` type by name when the program
//! starts (`cx.all_action_names()`) and can build one from a name plus JSON data, so this is a
//! thin view over that table rather than a second table: a crate adds an action by declaring it
//! (`#[action(namespace = table)]`), never by calling into this module.
//!
//! The view adds what the keymap, the palette (E11) and the agent tools need:
//! - [`ActionRegistry::namespaces`] and [`ActionRegistry::names_in`], so the palette can group
//!   and the validator can say "did you mean" within a namespace,
//! - [`ActionRegistry::command`], the action-to-`Command` mapping of non-negotiable 4: the
//!   action name *is* the [`CommandId`], so a binding dispatches the same command the palette
//!   and the MCP tool run, with no second behaviour behind the key,
//! - [`ActionRegistry::build`], which reports an unknown name or bad data as a value, never a
//!   panic.

use std::collections::BTreeMap;

use gpui::{Action, ActionBuildError, App};
use oxikube_domain::command::{self, CommandId, CommandMeta};
use serde_json::Value;

/// A snapshot of the action names GPUI has registered, indexed by namespace.
#[derive(Clone, Debug, Default)]
pub struct ActionRegistry {
    by_namespace: BTreeMap<&'static str, Vec<&'static str>>,
}

/// Why an action could not be built from a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildActionError {
    /// No action is registered under the name.
    Unknown,
    /// The action exists but rejected the data.
    InvalidData(String),
}

/// The namespace of an action name (`table` for `table::SelectNext`); `""` when it has none.
pub fn namespace_of(name: &str) -> &str {
    name.rsplit_once("::")
        .map_or("", |(namespace, _)| namespace)
}

impl ActionRegistry {
    /// Snapshot the actions registered with GPUI. Cheap enough to take per reload (a few
    /// hundred names); take a new one when actions can be registered later (extensions).
    pub fn from_app(cx: &App) -> Self {
        let mut by_namespace: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
        for name in cx.all_action_names() {
            let namespace = namespace_of(name);
            by_namespace.entry(namespace).or_default().push(name);
        }
        for names in by_namespace.values_mut() {
            names.sort_unstable();
        }
        Self { by_namespace }
    }

    /// The namespaces that have at least one action, sorted (`""` holds un-namespaced ones).
    pub fn namespaces(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_namespace.keys().copied()
    }

    /// The action names in `namespace`, sorted.
    pub fn names_in(&self, namespace: &str) -> &[&'static str] {
        self.by_namespace.get(namespace).map_or(&[], Vec::as_slice)
    }

    /// Every action name, sorted by namespace then name.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_namespace.values().flatten().copied()
    }

    /// Whether `name` is a registered action.
    pub fn contains(&self, name: &str) -> bool {
        self.by_namespace
            .get(namespace_of(name))
            .is_some_and(|names| names.binary_search(&name).is_ok())
    }

    /// The `Command` the action stands for, if the name is a declared [`CommandId`].
    ///
    /// Actions that are not commands (focus movement, menu navigation, window chrome) return
    /// `None`; their handlers are plain GPUI listeners.
    pub fn command(name: &str) -> Option<&'static CommandMeta> {
        command::lookup_str(name)
    }

    /// The commands `name` stands for: the command of that id when it is one, else the commands
    /// of the view action it names ([`stands_for`](crate::stands_for)). Empty for a plain
    /// navigation action.
    pub fn commands_of(name: &str) -> Vec<CommandId> {
        match Self::command(name) {
            Some(meta) => vec![meta.id],
            None => crate::stands_for::commands_of_action(name).collect(),
        }
    }

    /// Every declared command that has no registered action, so no key can be bound to it yet.
    /// A binary's start-up check can log these: a command nobody can bind is a missing wiring.
    pub fn commands_without_action(&self) -> Vec<CommandId> {
        command::COMMANDS
            .iter()
            .map(|meta| meta.id)
            .filter(|id| !self.contains(id.as_str()))
            .collect()
    }

    /// Build the action `name` with optional JSON `data`.
    pub fn build(
        cx: &App,
        name: &str,
        data: Option<Value>,
    ) -> Result<Box<dyn Action>, BuildActionError> {
        cx.build_action(name, data).map_err(|err| match err {
            ActionBuildError::NotFound { .. } => BuildActionError::Unknown,
            ActionBuildError::BuildError { error, .. } => {
                BuildActionError::InvalidData(error.to_string())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_is_everything_before_the_last_separator() {
        assert_eq!(namespace_of("table::SelectNext"), "table");
        assert_eq!(namespace_of("a::b::C"), "a::b");
        assert_eq!(namespace_of("Quit"), "");
    }

    #[test]
    fn a_command_id_is_its_own_action_name() {
        let meta = ActionRegistry::command("palette::Toggle").expect("declared command");
        assert_eq!(meta.id, CommandId::PALETTE_TOGGLE);
        assert!(ActionRegistry::command("table::SelectNext").is_none());
    }
}
