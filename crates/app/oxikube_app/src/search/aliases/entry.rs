//! The vocabulary of the alias table: [`AliasEntry`], [`AliasSource`], [`Resolution`] and the
//! [`AliasConflict`]s the help overlay lists.

use std::sync::Arc;

use oxikube_domain::AliasTarget;

/// Where an alias comes from. The order is the precedence: a lower variant hides a higher one
/// (`User` over `BuiltIn` over `Discovery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AliasSource {
    /// The user's `aliases.json`.
    User,
    /// The k9s-style table shipped with the app (`po`, `dp`, `svc`, ...).
    BuiltIn,
    /// The cluster's API discovery: plural, singular, short names and Kind of every served type.
    Discovery,
}

impl AliasSource {
    /// A short word for logs and the help overlay.
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::BuiltIn => "built-in",
            Self::Discovery => "discovery",
        }
    }
}

/// One name and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasEntry {
    /// The alias, lower-case (lookups ignore case).
    pub name: Arc<str>,
    /// Where it leads.
    pub target: AliasTarget,
    /// Which layer it came from.
    pub source: AliasSource,
}

/// The answer to [`AliasTable::resolve`](super::AliasTable::resolve).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The name leads to one place (a higher layer may have shadowed others: see
    /// [`AliasTable::conflicts`](super::AliasTable::conflicts)).
    Exact(AliasEntry),
    /// Several served types claim the name and no higher layer settles it (two CRDs in different
    /// groups with the same plural). The candidates are in the table's fixed order, core group
    /// first and then by group name, so [`Resolution::target`] is the same on every run; a UI
    /// may offer the rest.
    Ambiguous(Arc<[AliasEntry]>),
    /// Nothing is called this. `suggestions` are close names, best first (at most
    /// [`MAX_SUGGESTIONS`](super::MAX_SUGGESTIONS)).
    Unknown {
        /// Names that start with the input or are a few edits away from it.
        suggestions: Vec<Arc<str>>,
    },
}

impl Resolution {
    /// Where the name leads: the entry's target, or the first candidate of an ambiguous name.
    pub fn target(&self) -> Option<&AliasTarget> {
        match self {
            Self::Exact(entry) => Some(&entry.target),
            Self::Ambiguous(candidates) => candidates.first().map(|entry| &entry.target),
            Self::Unknown { .. } => None,
        }
    }

    /// Whether the name is known (exact or ambiguous).
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown { .. })
    }
}

/// Why a name appears in the conflict list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// A higher layer holds the name and hides entries of a lower one that lead elsewhere (the
    /// user's `po` over the built-in, a CRD's short name `dp` under the built-in).
    Shadowed,
    /// Several entries of the discovery layer lead to different types; the first is what
    /// [`Resolution::target`] gives.
    Ambiguous,
}

/// A name that more than one entry claims with different destinations. Never silent: the table
/// resolves it by its fixed rules and lists it here for the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasConflict {
    /// The contested name.
    pub name: Arc<str>,
    /// How it was settled.
    pub kind: ConflictKind,
    /// The entry that wins (for [`ConflictKind::Ambiguous`], the first candidate).
    pub winner: AliasEntry,
    /// The entries that lose, or the other candidates.
    pub others: Vec<AliasEntry>,
}
