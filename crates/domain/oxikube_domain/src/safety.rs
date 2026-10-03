//! Safety vocabulary shared by commands, the `MutationGuard` and the audit
//! log: [`Risk`], [`ConfirmTier`] and [`Initiator`].
//!
//! These are plain data. The pipeline that applies them (read-only check,
//! confirmation, dry-run, execute, audit) lives in `oxikube_app::mutation`;
//! see ADR 0012.
//!
//! # Risk to confirmation tier
//!
//! | [`Risk`] | [`ConfirmTier`] | Extra |
//! |---|---|---|
//! | `Low` | `Simple` | one-click confirm; a session setting may skip it |
//! | `Medium` | `Simple` | the dialog names the target and the cluster |
//! | `High` | `TypeName` | the user types the resource name |
//! | `Irreversible` | `TypeName` | plus a mandatory server dry-run diff |
//!
//! Reads never confirm ([`ConfirmTier::None`]) and carry no risk.

use std::fmt;

use serde::{Deserialize, Serialize};

/// How much friction a mutating action asks for before it runs.
///
/// Ordered by friction: `None < Simple < TypeName`, so the guard can take the
/// maximum of a command's static tier and a tier raised by the target (for
/// example a namespace or node).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmTier {
    /// No confirmation (reads and local UI actions).
    #[default]
    None,
    /// A confirm dialog naming the target and the cluster.
    Simple,
    /// The user must type the resource name to proceed.
    TypeName,
}

impl ConfirmTier {
    /// Every tier, least to most friction.
    pub const ALL: [ConfirmTier; 3] = [
        ConfirmTier::None,
        ConfirmTier::Simple,
        ConfirmTier::TypeName,
    ];

    /// Stable lowercase name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ConfirmTier::None => "none",
            ConfirmTier::Simple => "simple",
            ConfirmTier::TypeName => "type_name",
        }
    }
}

impl fmt::Display for ConfirmTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Blast radius of a mutating action. Stories say "confirm low / medium /
/// high"; [`Risk::confirm_tier`] is the single mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Easily undone, small blast radius (uncordon a node).
    Low,
    /// Disruptive but recoverable (scale, restart, delete one pod).
    Medium,
    /// Hard to undo or wide blast radius (delete a resource, drain a node).
    High,
    /// Cannot be undone at all (cascading deletes of data-bearing objects).
    Irreversible,
}

impl Risk {
    /// Every risk level, lowest first.
    pub const ALL: [Risk; 4] = [Risk::Low, Risk::Medium, Risk::High, Risk::Irreversible];

    /// The confirmation tier this risk maps to (see the module table).
    pub const fn confirm_tier(self) -> ConfirmTier {
        match self {
            Risk::Low | Risk::Medium => ConfirmTier::Simple,
            Risk::High | Risk::Irreversible => ConfirmTier::TypeName,
        }
    }

    /// Whether the guard must show a server-side dry-run diff before running.
    pub const fn requires_dry_run_diff(self) -> bool {
        matches!(self, Risk::Irreversible)
    }

    /// Stable lowercase name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
            Risk::Irreversible => "irreversible",
        }
    }
}

impl fmt::Display for Risk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who asked for a mutation. Recorded in every audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Initiator {
    /// A direct UI gesture (button, context menu).
    Ui,
    /// A command dispatched through the command bus (palette, keymap).
    Command,
    /// A hosted agent through an MCP tool call.
    Agent,
    /// A WASM extension.
    Plugin,
}

impl Initiator {
    /// Every initiator.
    pub const ALL: [Initiator; 4] = [
        Initiator::Ui,
        Initiator::Command,
        Initiator::Agent,
        Initiator::Plugin,
    ];

    /// Stable lowercase name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Initiator::Ui => "ui",
            Initiator::Command => "command",
            Initiator::Agent => "agent",
            Initiator::Plugin => "plugin",
        }
    }
}

impl fmt::Display for Initiator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_are_ordered_by_friction() {
        assert!(ConfirmTier::None < ConfirmTier::Simple);
        assert!(ConfirmTier::Simple < ConfirmTier::TypeName);
        assert_eq!(ConfirmTier::default(), ConfirmTier::None);
        assert_eq!(
            ConfirmTier::Simple.max(ConfirmTier::TypeName),
            ConfirmTier::TypeName
        );
    }

    #[test]
    fn risk_maps_to_glossary_tiers() {
        assert_eq!(Risk::Low.confirm_tier(), ConfirmTier::Simple);
        assert_eq!(Risk::Medium.confirm_tier(), ConfirmTier::Simple);
        assert_eq!(Risk::High.confirm_tier(), ConfirmTier::TypeName);
        assert_eq!(Risk::Irreversible.confirm_tier(), ConfirmTier::TypeName);
        assert!(Risk::ALL.windows(2).all(|w| w[0] < w[1]));
        assert!(
            Risk::ALL
                .iter()
                .all(|r| r.requires_dry_run_diff() == (*r == Risk::Irreversible))
        );
    }

    #[test]
    fn names_match_serde_forms() {
        for t in ConfirmTier::ALL {
            assert_eq!(serde_json::to_string(&t).unwrap(), format!("\"{t}\""));
            let back: ConfirmTier = serde_json::from_str(&format!("\"{t}\"")).unwrap();
            assert_eq!(back, t);
        }
        for r in Risk::ALL {
            assert_eq!(serde_json::to_string(&r).unwrap(), format!("\"{r}\""));
        }
        for i in Initiator::ALL {
            assert_eq!(serde_json::to_string(&i).unwrap(), format!("\"{i}\""));
            let back: Initiator = serde_json::from_str(&format!("\"{i}\"")).unwrap();
            assert_eq!(back, i);
        }
    }
}
