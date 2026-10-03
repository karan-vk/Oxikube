//! What a session or backend can do: [`Capabilities`] (the bitflags set) and
//! [`Capability`] (one named flag).
//!
//! A command lists the capabilities it `needs`; the palette hides commands the
//! session cannot run and the guard rejects them. Every flag has a stable
//! lowercase name used in settings and tool schemas; [`Capabilities`] serialises
//! as a JSON array of those names (`["mutate","exec"]`), never as raw bits, so
//! adding a flag later is not a breaking change.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    /// A set of [`Capability`] flags. Bits are in-memory only and may be
    /// renumbered; persist the names.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
    pub struct Capabilities: u32 {
        /// Create, update, patch or delete cluster objects.
        const MUTATE = 1 << 0;
        /// Run commands in containers (`pods/exec`, `pods/attach`).
        const EXEC = 1 << 1;
        /// Read container logs.
        const LOGS = 1 << 2;
        /// Open port-forwards (`pods/portforward`).
        const PORTFORWARD = 1 << 3;
        /// Helm releases are available.
        const HELM = 1 << 4;
        /// An Argo CD backend is available.
        const ARGO = 1 << 5;
        /// Live metrics (metrics-server or Prometheus) are available.
        const METRICS = 1 << 6;
    }
}

impl Capabilities {
    /// The capabilities in `self` that `have` lacks (empty when satisfied).
    pub const fn missing_from(self, have: Capabilities) -> Capabilities {
        self.difference(have)
    }

    /// Whether `have` covers every capability in `self`.
    pub const fn satisfied_by(self, have: Capabilities) -> bool {
        have.contains(self)
    }

    /// The single [`Capability`] flags set in `self`, in declaration order.
    pub fn iter_capabilities(self) -> impl Iterator<Item = Capability> {
        Capability::ALL
            .into_iter()
            .filter(move |c| self.contains(c.flag()))
    }

    /// Build a set from stable names, failing on the first unknown name.
    pub fn from_names<'a>(
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, UnknownCapability> {
        let mut set = Capabilities::empty();
        for name in names {
            set |= name.parse::<Capability>()?.flag();
        }
        Ok(set)
    }

    /// Stable names of the flags set in `self`, in declaration order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        self.iter_capabilities().map(Capability::name)
    }
}

impl From<Capability> for Capabilities {
    fn from(value: Capability) -> Self {
        value.flag()
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Capabilities::empty(), |acc, c| acc | c.flag())
    }
}

impl Serialize for Capabilities {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter_capabilities())
    }
}

impl<'de> Deserialize<'de> for Capabilities {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let list = Vec::<Capability>::deserialize(deserializer)?;
        Ok(list.into_iter().collect())
    }
}

/// One capability, as a value (for iteration, names and schemas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    /// See [`Capabilities::MUTATE`].
    Mutate,
    /// See [`Capabilities::EXEC`].
    Exec,
    /// See [`Capabilities::LOGS`].
    Logs,
    /// See [`Capabilities::PORTFORWARD`].
    PortForward,
    /// See [`Capabilities::HELM`].
    Helm,
    /// See [`Capabilities::ARGO`].
    Argo,
    /// See [`Capabilities::METRICS`].
    Metrics,
}

impl Capability {
    /// Every capability, in declaration order.
    pub const ALL: [Capability; 7] = [
        Capability::Mutate,
        Capability::Exec,
        Capability::Logs,
        Capability::PortForward,
        Capability::Helm,
        Capability::Argo,
        Capability::Metrics,
    ];

    /// The matching single-bit [`Capabilities`] flag.
    pub const fn flag(self) -> Capabilities {
        match self {
            Capability::Mutate => Capabilities::MUTATE,
            Capability::Exec => Capabilities::EXEC,
            Capability::Logs => Capabilities::LOGS,
            Capability::PortForward => Capabilities::PORTFORWARD,
            Capability::Helm => Capabilities::HELM,
            Capability::Argo => Capabilities::ARGO,
            Capability::Metrics => Capabilities::METRICS,
        }
    }

    /// Stable lowercase name, identical to the serde form. Renaming one is a
    /// breaking change for settings and tool schemas.
    pub const fn name(self) -> &'static str {
        match self {
            Capability::Mutate => "mutate",
            Capability::Exec => "exec",
            Capability::Logs => "logs",
            Capability::PortForward => "portforward",
            Capability::Helm => "helm",
            Capability::Argo => "argo",
            Capability::Metrics => "metrics",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A capability name that is not in [`Capability::ALL`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown capability {0:?}")]
pub struct UnknownCapability(pub String);

impl FromStr for Capability {
    type Err = UnknownCapability;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Capability::ALL
            .into_iter()
            .find(|c| c.name() == s)
            .ok_or_else(|| UnknownCapability(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_contains_and_difference() {
        let rw = Capabilities::MUTATE | Capabilities::EXEC;
        assert!(rw.contains(Capabilities::MUTATE));
        assert!(!rw.contains(Capabilities::LOGS));
        assert!(rw.contains(Capabilities::empty()));
        assert_eq!(rw.missing_from(Capabilities::MUTATE), Capabilities::EXEC);
        assert!(rw.satisfied_by(Capabilities::all()));
        assert!(!rw.satisfied_by(Capabilities::MUTATE));
        assert!(Capabilities::empty().satisfied_by(Capabilities::empty()));
    }

    #[test]
    fn every_capability_has_exactly_one_distinct_bit() {
        let mut seen = Capabilities::empty();
        for c in Capability::ALL {
            assert_eq!(c.flag().bits().count_ones(), 1, "{c}");
            assert!(!seen.intersects(c.flag()), "{c} reuses a bit");
            seen |= c.flag();
        }
        assert_eq!(seen, Capabilities::all(), "ALL misses a declared flag");
    }

    #[test]
    fn names_round_trip() {
        for c in Capability::ALL {
            assert_eq!(c.name().parse::<Capability>().unwrap(), c);
            assert_eq!(c.to_string(), c.name());
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json, format!("\"{}\"", c.name()));
        }
        let all = Capabilities::all();
        assert_eq!(Capabilities::from_names(all.names()).unwrap(), all);
        assert_eq!(
            "nope".parse::<Capability>(),
            Err(UnknownCapability("nope".into()))
        );
        assert!(Capabilities::from_names(["mutate", "nope"]).is_err());
    }

    #[test]
    fn serde_is_a_list_of_names() {
        let set = Capabilities::LOGS | Capabilities::MUTATE;
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"["mutate","logs"]"#);
        let back: Capabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(back, set);
        let empty: Capabilities = serde_json::from_str("[]").unwrap();
        assert!(empty.is_empty());
        assert!(serde_json::from_str::<Capabilities>(r#"["bogus"]"#).is_err());
        // Duplicates and order do not matter.
        let dup: Capabilities = serde_json::from_str(r#"["logs","logs","mutate"]"#).unwrap();
        assert_eq!(dup, set);
    }
}
