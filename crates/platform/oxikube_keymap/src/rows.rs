//! The flat list the keymap editor shows (E21-S07): one [`KeymapRow`] per binding of every layer,
//! plus one row per command that nothing is bound to.
//!
//! The rows are computed from the layers' *sections* ([`LayerBinding`]), not from GPUI's keymap:
//! GPUI's keymap forgets what a `null` hid and which binding another replaced, and the editor
//! must show exactly that ("overridden", "unbound by you"). The builder is a pure function of the
//! bindings and a predicate for "is this action registered", so it is tested without a window and
//! costs well under a millisecond for the shipped keymap.
//!
//! Precedence is the keymap's own: layers apply default, vim, user, sections and bindings in file
//! order, and the last binding of the same keystrokes in the same context wins. A binding it
//! replaced stays in the list, marked [`RowStatus::Overridden`], or
//! [`RowStatus::UnboundByUser`] when the replacement is a `null` of the user's file.

use std::collections::{HashMap, HashSet};

use oxikube_domain::command::{self, CommandId};
use serde_json::Value;

use crate::conflicts::normalise;
use crate::file::KeymapAction;
use crate::layer::KeymapLayer;
use crate::stands_for;

/// One binding as a layer's file states it, in precedence order (weakest first).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerBinding {
    /// The layer the binding is in.
    pub layer: KeymapLayer,
    /// The section's context expression, whitespace collapsed; `None` for everywhere.
    pub context: Option<String>,
    /// The keystrokes as the file writes them.
    pub keystrokes: String,
    /// What the binding does.
    pub action: KeymapAction,
}

/// Where a row stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStatus {
    /// The binding is in force.
    Active,
    /// A binding of the same keys and context that comes later (usually from a stronger layer)
    /// replaced it.
    Overridden,
    /// The user's file has a `null` for the same keys and context: no action runs on them.
    UnboundByUser,
    /// A command nothing is bound to; the row has no keystrokes.
    Unbound,
}

/// One line of the keymap editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapRow {
    /// The action's registered name (`resource_table::ViewYaml`); a command's id for commands.
    pub action: String,
    /// What a person reads: the command's title, or the action name put into words.
    pub title: String,
    /// The key-context expression the binding is limited to; `None` for everywhere.
    pub context: Option<String>,
    /// The keystrokes, each in the canonical spelling (`ctrl-k ctrl-s` is two); empty when unbound.
    pub keystrokes: Vec<String>,
    /// The JSON data of an action that takes it, as compact text.
    pub data: Option<String>,
    /// The layer the binding is from; `None` for an unbound command.
    pub source: Option<KeymapLayer>,
    /// Whether the binding is in force.
    pub status: RowStatus,
}

impl KeymapRow {
    /// The keystrokes joined by spaces (`ctrl-k ctrl-s`); empty when unbound.
    pub fn keystrokes_text(&self) -> String {
        self.keystrokes.join(" ")
    }
}

/// The rows for `bindings` (weakest first), skipping the actions `installable` rejects (with
/// their data), as the loader does: the embedded layers name actions of crates absent from a
/// build, and a user binding to an unknown action or with bad data is reported, not installed.
/// A skipped binding replaces nothing. Adds a row for every declared command with no active
/// binding.
pub fn build_rows(
    bindings: &[LayerBinding],
    installable: &dyn Fn(&str, Option<&Value>) -> bool,
) -> Vec<KeymapRow> {
    let installed = |binding: &LayerBinding| match &binding.action {
        KeymapAction::Unbind => true,
        KeymapAction::Action { name, data } => installable(name, data.as_ref()),
    };
    // The last installed binding of each (context, keystrokes) wins, a `null` included.
    let mut winner: HashMap<(Option<&str>, String), usize> = HashMap::new();
    for (index, binding) in bindings.iter().enumerate() {
        if installed(binding) {
            winner.insert(
                (binding.context.as_deref(), normalise(&binding.keystrokes)),
                index,
            );
        }
    }
    let mut rows = Vec::new();
    let mut active: HashSet<&str> = HashSet::new();
    for (index, binding) in bindings.iter().enumerate() {
        let KeymapAction::Action { name, data } = &binding.action else {
            continue;
        };
        if !installed(binding) {
            continue;
        }
        let key = (binding.context.as_deref(), normalise(&binding.keystrokes));
        let won = winner[&key];
        let status = if won == index {
            active.insert(name);
            RowStatus::Active
        } else if bindings[won].layer == KeymapLayer::User
            && bindings[won].action == KeymapAction::Unbind
        {
            RowStatus::UnboundByUser
        } else {
            RowStatus::Overridden
        };
        rows.push(KeymapRow {
            action: name.clone(),
            title: title_of(name),
            context: binding.context.clone(),
            keystrokes: key.1.split_whitespace().map(str::to_owned).collect(),
            data: data.as_ref().map(ToString::to_string),
            source: Some(binding.layer),
            status,
        });
    }
    for meta in command::COMMANDS {
        let id = meta.id;
        let bound = active.contains(id.as_str())
            || stands_for::view_actions_of(id).any(|action| active.contains(action));
        if !bound {
            rows.push(unbound_row(id, meta.title));
        }
    }
    // Lowercase each title once, not once per comparison.
    rows.sort_by_cached_key(|row| {
        (
            row.title.to_lowercase(),
            row.action.clone(),
            row.context.clone(),
            row.keystrokes.clone(),
        )
    });
    rows
}

fn unbound_row(id: CommandId, title: &str) -> KeymapRow {
    KeymapRow {
        action: id.as_str().to_owned(),
        title: title.to_owned(),
        context: None,
        keystrokes: Vec::new(),
        data: None,
        source: None,
        status: RowStatus::Unbound,
    }
}

/// The title of an action: its command's, else the command a view action stands for, else the
/// name put into words (`resource_table::ViewYaml` is "Resource Table: View Yaml").
pub fn title_of(action: &str) -> String {
    if let Some(meta) = command::lookup_str(action) {
        return meta.title.to_owned();
    }
    if let Some(id) = stands_for::commands_of_action(action).next()
        && let Some(meta) = command::lookup(id)
    {
        return meta.title.to_owned();
    }
    humanise(action)
}

fn humanise(action: &str) -> String {
    let (namespace, name) = action.rsplit_once("::").unwrap_or(("", action));
    let words = |text: &str| {
        let mut out = String::new();
        let mut previous_lower = false;
        for ch in text.chars() {
            if ch == '_' {
                out.push(' ');
                previous_lower = false;
                continue;
            }
            if ch.is_uppercase() && previous_lower {
                out.push(' ');
            }
            if out.is_empty() || out.ends_with(' ') {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            previous_lower = ch.is_lowercase();
        }
        out
    };
    if namespace.is_empty() {
        words(name)
    } else {
        format!("{}: {}", words(namespace), words(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        layer: KeymapLayer,
        context: Option<&str>,
        keys: &str,
        to: Option<&str>,
    ) -> LayerBinding {
        LayerBinding {
            layer,
            context: context.map(str::to_owned),
            keystrokes: keys.to_owned(),
            action: to.map_or(KeymapAction::Unbind, |name| KeymapAction::Action {
                name: name.to_owned(),
                data: None,
            }),
        }
    }

    fn build(bindings: &[LayerBinding]) -> Vec<KeymapRow> {
        build_rows(bindings, &|_, _| true)
    }

    fn row<'a>(rows: &'a [KeymapRow], action: &str) -> &'a KeymapRow {
        rows.iter()
            .find(|row| row.action == action && row.status != RowStatus::Unbound)
            .expect("a bound row")
    }

    #[test]
    fn a_binding_is_one_row_with_canonical_keys_and_its_layer() {
        let rows = build(&[binding(
            KeymapLayer::Default,
            Some("Workspace"),
            "shift-ctrl-a",
            Some("x::Do"),
        )]);
        let row = row(&rows, "x::Do");
        assert_eq!(row.keystrokes, ["ctrl-shift-a"]);
        assert_eq!(row.context.as_deref(), Some("Workspace"));
        assert_eq!(row.source, Some(KeymapLayer::Default));
        assert_eq!(row.status, RowStatus::Active);
        assert_eq!(row.title, "X: Do");
    }

    #[test]
    fn a_sequence_keeps_each_keystroke() {
        let rows = build(&[binding(
            KeymapLayer::Default,
            None,
            "ctrl-k ctrl-s",
            Some("x::Do"),
        )]);
        assert_eq!(row(&rows, "x::Do").keystrokes, ["ctrl-k", "ctrl-s"]);
    }

    #[test]
    fn a_user_binding_overrides_the_default_on_the_same_keys_and_context() {
        let rows = build(&[
            binding(KeymapLayer::Default, Some("A"), "ctrl-a", Some("x::Old")),
            binding(KeymapLayer::Default, Some("B"), "ctrl-a", Some("x::Other")),
            binding(KeymapLayer::User, Some("A"), "ctrl-a", Some("x::New")),
        ]);
        assert_eq!(row(&rows, "x::Old").status, RowStatus::Overridden);
        assert_eq!(row(&rows, "x::New").status, RowStatus::Active);
        assert_eq!(row(&rows, "x::New").source, Some(KeymapLayer::User));
        assert_eq!(
            row(&rows, "x::Other").status,
            RowStatus::Active,
            "another context"
        );
    }

    #[test]
    fn a_user_null_marks_the_default_unbound_by_user_and_has_no_row_of_its_own() {
        let rows = build(&[
            binding(KeymapLayer::Default, None, "ctrl-a", Some("x::Old")),
            binding(KeymapLayer::User, None, "ctrl-a", None),
            binding(KeymapLayer::User, None, "ctrl-z", None),
        ]);
        assert_eq!(row(&rows, "x::Old").status, RowStatus::UnboundByUser);
        assert!(rows.iter().all(|row| row.keystrokes_text() != "ctrl-z"));
    }

    #[test]
    fn a_vim_null_is_an_override_not_the_users() {
        let rows = build(&[
            binding(KeymapLayer::Default, None, "j", Some("x::Down")),
            binding(KeymapLayer::Vim, None, "j", None),
        ]);
        assert_eq!(row(&rows, "x::Down").status, RowStatus::Overridden);
    }

    #[test]
    fn unregistered_actions_are_left_out() {
        let rows = build_rows(
            &[binding(
                KeymapLayer::Default,
                None,
                "ctrl-a",
                Some("x::Gone"),
            )],
            &|_, _| false,
        );
        assert!(rows.iter().all(|row| row.action != "x::Gone"));
    }

    #[test]
    fn a_binding_the_loader_rejects_replaces_nothing() {
        let rows = build_rows(
            &[
                binding(KeymapLayer::Default, Some("A"), "ctrl-a", Some("x::Real")),
                binding(KeymapLayer::User, Some("A"), "ctrl-a", Some("x::Typo")),
            ],
            &|name, _| name != "x::Typo",
        );
        assert_eq!(row(&rows, "x::Real").status, RowStatus::Active);
        assert!(rows.iter().all(|row| row.action != "x::Typo"));
    }

    #[test]
    fn a_command_without_an_active_binding_gets_an_unbound_row() {
        let id = command::COMMANDS[0].id;
        let rows = build(&[]);
        let unbound = rows
            .iter()
            .find(|row| row.action == id.as_str())
            .expect("every command is listed");
        assert_eq!(unbound.status, RowStatus::Unbound);
        assert!(unbound.keystrokes.is_empty() && unbound.source.is_none());

        let rows = build(&[binding(
            KeymapLayer::Default,
            None,
            "ctrl-a",
            Some(id.as_str()),
        )]);
        assert!(
            rows.iter()
                .filter(|row| row.action == id.as_str())
                .all(|row| row.status == RowStatus::Active),
            "a bound command has no unbound row"
        );
    }

    #[test]
    fn a_command_whose_only_binding_was_replaced_is_listed_unbound_too() {
        let id = command::COMMANDS[0].id;
        let rows = build(&[
            binding(KeymapLayer::Default, None, "ctrl-a", Some(id.as_str())),
            binding(KeymapLayer::User, None, "ctrl-a", None),
        ]);
        let statuses: Vec<_> = rows
            .iter()
            .filter(|row| row.action == id.as_str())
            .map(|row| row.status)
            .collect();
        assert!(statuses.contains(&RowStatus::UnboundByUser));
        assert!(statuses.contains(&RowStatus::Unbound));
    }

    #[test]
    fn titles_come_from_the_command_then_from_the_name() {
        assert_eq!(title_of("keymap::OpenUser"), "Open User Keymap");
        assert_eq!(
            title_of("resource_table::SelectNextRow"),
            "Resource Table: Select Next Row"
        );
        assert_eq!(title_of("Quit"), "Quit");
    }

    #[test]
    fn rows_are_sorted_by_title() {
        let rows = build(&[
            binding(KeymapLayer::Default, None, "a", Some("z::Zed")),
            binding(KeymapLayer::Default, None, "b", Some("a::Alpha")),
        ]);
        let at = |action: &str| rows.iter().position(|r| r.action == action).unwrap();
        assert!(at("a::Alpha") < at("z::Zed"));
    }
}
