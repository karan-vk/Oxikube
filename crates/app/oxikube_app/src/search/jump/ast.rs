//! The syntax tree of a jump line: [`JumpCommand`] and what it holds.
//!
//! The tree is purely syntactic. `deploy` is a word here, not yet the Deployments type; the
//! namespace is a name, not yet checked against the cluster. [`plan`](super::plan) resolves it
//! against the alias table, the namespaces and the contexts, so the same line can be parsed on
//! every keystroke (to underline what is wrong so far) without touching either.
//!
//! Printing a tree ([`std::fmt::Display`]) gives the canonical form of the line (`pods
//! kube-system /api app=x @prod`, tokens in a fixed order, reserved words in their short form),
//! which parses back to an equal tree; that is what the history stores.

use std::fmt;

use super::span::Spanned;

/// The text of a `/` filter, uninterpreted: what follows the slash (`api`, `!api`, `-l app=x`,
/// `-f fz`). The filter bar owns its grammar (`oxikube_app::store::filter`), so a bad regex is
/// reported there, where the user can fix it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RawFilter(String);

impl RawFilter {
    /// A filter with this text (without the leading `/`).
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text after the slash.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RawFilter {
    /// The filter as typed, with its slash.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.0)
    }
}

/// A step through the history of jump commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryStep {
    /// `-`: the previous view, and back again on a second use.
    Last,
    /// `[`: one command back in the history.
    Back,
    /// `]`: one command forward in the history.
    Forward,
}

impl HistoryStep {
    /// The word that stands for the step in the bar.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Last => "-",
            Self::Back => "[",
            Self::Forward => "]",
        }
    }
}

/// `:<alias> [ns] [/filter] [k=v,..] [@ctx]`: the list of a resource type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceJump {
    /// The alias, as typed (`po`, `deploy`, `certs`, `widgets.example.io`).
    pub alias: Spanned<String>,
    /// The namespace to show (`all` for every one).
    pub namespace: Option<Spanned<String>>,
    /// The `/filter`.
    pub filter: Option<Spanned<RawFilter>>,
    /// The label selector, `k=v,k2=v2`.
    pub labels: Option<Spanned<String>>,
    /// The cluster context, without the `@`.
    pub context: Option<Spanned<String>>,
}

/// A parsed jump line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpCommand {
    /// A resource list: `:pods`, `:deploy kube-system /api app=x @prod`.
    Resource(ResourceJump),
    /// `:ctx [name]`: switch to a cluster context, or list them.
    Ctx(Option<Spanned<String>>),
    /// `:ns [name]`: the namespaces list, or select one.
    Ns(Option<Spanned<String>>),
    /// `:q`: quit.
    Quit,
    /// `-`, `[`, `]`: move through the history.
    History(HistoryStep),
}

impl fmt::Display for ResourceJump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.alias.value)?;
        if let Some(ns) = &self.namespace {
            write!(f, " {}", ns.value)?;
        }
        if let Some(filter) = &self.filter {
            write!(f, " {}", filter.value)?;
        }
        if let Some(labels) = &self.labels {
            write!(f, " {}", labels.value)?;
        }
        if let Some(ctx) = &self.context {
            write!(f, " @{}", ctx.value)?;
        }
        Ok(())
    }
}

impl fmt::Display for JumpCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource(resource) => resource.fmt(f),
            Self::Ctx(None) => f.write_str("ctx"),
            Self::Ctx(Some(name)) => write!(f, "ctx {}", name.value),
            Self::Ns(None) => f.write_str("ns"),
            Self::Ns(Some(name)) => write!(f, "ns {}", name.value),
            Self::Quit => f.write_str("q"),
            Self::History(step) => f.write_str(step.symbol()),
        }
    }
}

impl JumpCommand {
    /// Whether running the command goes into the history (`-`, `[`, `]` and `:q` do not).
    pub fn is_recorded(&self) -> bool {
        !matches!(self, Self::History(_) | Self::Quit)
    }
}
