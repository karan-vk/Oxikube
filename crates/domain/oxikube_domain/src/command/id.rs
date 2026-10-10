//! [`CommandId`]: the stable `namespace::Verb` name of a command.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::registry;

/// Stable identifier of a command, written `namespace::Verb`
/// (`pod::Delete`, `workload::Scale`, `cluster::ToggleReadOnly`).
///
/// The same string is the keymap action name and the source of the MCP tool
/// name ([`CommandId::tool_name`]), so renaming an id is a breaking change for
/// user keymaps and agent prompts.
///
/// The id wraps a `&'static str`: copying is free and palette lookups never
/// allocate. Ids are validated at compile time when built in a `const`
/// context ([`CommandId::new`] panics on a malformed id), and deserialising
/// accepts only ids present in the registry ([`COMMANDS`](super::COMMANDS)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandId(&'static str);

/// Whether `s` is `namespace::Verb`: a lowercase ASCII namespace
/// (`[a-z][a-z0-9]*`), the separator `::`, and an upper-camel verb
/// (`[A-Z][A-Za-z0-9]*`).
pub const fn is_well_formed(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || !b[0].is_ascii_lowercase() {
        return false;
    }
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_lowercase() || b[i].is_ascii_digit()) {
        i += 1;
    }
    if i + 2 >= b.len() || b[i] != b':' || b[i + 1] != b':' {
        return false;
    }
    i += 2;
    if !b[i].is_ascii_uppercase() {
        return false;
    }
    i += 1;
    while i < b.len() {
        if !b[i].is_ascii_alphanumeric() {
            return false;
        }
        i += 1;
    }
    true
}

impl CommandId {
    /// Wrap a `namespace::Verb` literal.
    ///
    /// # Panics
    ///
    /// Panics (a compile error in `const` context) if `id` is not
    /// [well formed](is_well_formed).
    pub const fn new(id: &'static str) -> Self {
        assert!(is_well_formed(id), "command id must be `namespace::Verb`");
        Self(id)
    }

    /// The id as `namespace::Verb`.
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// The part before `::` (`pod`).
    pub fn namespace(self) -> &'static str {
        self.0.split_once("::").map_or(self.0, |(ns, _)| ns)
    }

    /// The part after `::` (`Delete`).
    pub fn verb(self) -> &'static str {
        self.0.split_once("::").map_or("", |(_, verb)| verb)
    }

    /// The MCP tool name for this command.
    ///
    /// The rule is deterministic so keymap action, command and tool stay 1:1:
    ///
    /// | id namespace | tool name | example |
    /// |---|---|---|
    /// | `helm`, `argo` | `<ns>.<verb_snake>` | `helm::Rollback` -> `helm.rollback` |
    /// | app-level (`cluster`, `namespace`, `kubeconfig`, `view`, `palette`, `jump`, `help`, `keymap`, `settings`, `app`, `window`, `terminal`) | `app.<ns>_<verb_snake>` | `cluster::ToggleReadOnly` -> `app.cluster_toggle_read_only` |
    /// | everything else (resource verbs) | `k8s.<ns>_<verb_snake>` | `workload::Scale` -> `k8s.workload_scale` |
    ///
    /// Extension tools (`ext.<id>.*`) are not commands and do not go through
    /// this mapping.
    pub fn tool_name(self) -> String {
        let ns = self.namespace();
        let verb = snake_case(self.verb());
        match ns {
            "helm" | "argo" => format!("{ns}.{verb}"),
            "cluster" | "namespace" | "kubeconfig" | "view" | "palette" | "jump" | "help"
            | "keymap"
            | "settings" | "app" | "window" | "terminal" => {
                format!("app.{ns}_{verb}")
            }
            _ => format!("k8s.{ns}_{verb}"),
        }
    }
}

/// `ToggleReadOnly` -> `toggle_read_only`.
fn snake_case(upper_camel: &str) -> String {
    let mut out = String::with_capacity(upper_camel.len() + 4);
    for (i, ch) in upper_camel.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<str> for CommandId {
    fn as_ref(&self) -> &str {
        self.0
    }
}

/// A string that is not the id of any registered command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown command id {0:?}")]
pub struct UnknownCommandId(pub String);

impl FromStr for CommandId {
    type Err = UnknownCommandId;

    /// Resolve a registered id (keymap action names, tool arguments).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        registry::lookup_str(s)
            .map(|meta| meta.id)
            .ok_or_else(|| UnknownCommandId(s.to_owned()))
    }
}

impl Serialize for CommandId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for CommandId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_rule() {
        for ok in ["pod::Delete", "cluster::ToggleReadOnly", "k8s2::Do1"] {
            assert!(is_well_formed(ok), "{ok}");
        }
        for bad in [
            "",
            "pod",
            "pod::",
            "::Delete",
            "pod::delete",
            "Pod::Delete",
            "pod:Delete",
            "pod::Delete::Now",
            "pod::Del ete",
            "pod::Del_ete",
            "po-d::Delete",
            "2pod::Delete",
            "pod:::Delete",
        ] {
            assert!(!is_well_formed(bad), "{bad}");
        }
    }

    #[test]
    fn parts_and_tool_names() {
        let id = CommandId::new("cluster::ToggleReadOnly");
        assert_eq!(id.namespace(), "cluster");
        assert_eq!(id.verb(), "ToggleReadOnly");
        assert_eq!(id.tool_name(), "app.cluster_toggle_read_only");
        assert_eq!(
            CommandId::new("workload::Scale").tool_name(),
            "k8s.workload_scale"
        );
        assert_eq!(
            CommandId::new("pod::PortForward").tool_name(),
            "k8s.pod_port_forward"
        );
        assert_eq!(
            CommandId::new("helm::Rollback").tool_name(),
            "helm.rollback"
        );
        assert_eq!(
            CommandId::new("argo::HardRefresh").tool_name(),
            "argo.hard_refresh"
        );
        assert_eq!(
            CommandId::new("terminal::OpenLink").tool_name(),
            "app.terminal_open_link"
        );
    }

    #[test]
    #[should_panic(expected = "namespace::Verb")]
    fn new_rejects_malformed() {
        let _ = CommandId::new("pod.delete");
    }

    #[test]
    fn serde_accepts_only_registered_ids() {
        let id = CommandId::new("pod::Delete");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"pod::Delete\"");
        assert_eq!(
            serde_json::from_str::<CommandId>("\"pod::Delete\"").unwrap(),
            id
        );
        assert!(serde_json::from_str::<CommandId>("\"pod::Explode\"").is_err());
        assert!(serde_json::from_str::<CommandId>("\"nonsense\"").is_err());
    }

    #[test]
    fn ordering_is_by_string() {
        assert!(CommandId::new("cluster::Select") < CommandId::new("pod::Delete"));
        assert!(CommandId::new("resource::Open") < CommandId::new("resource::OpenList"));
    }
}
