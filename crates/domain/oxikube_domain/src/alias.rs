//! [`AliasTarget`]: what a jump-bar name (`:deploy`, `:certs`, a user's `:prodpods`) stands for.
//!
//! The alias *table* (layers, precedence, collisions) is `oxikube_app::search::aliases`; this is
//! only the vocabulary shared by that table and the user's `aliases.json`, which the platform
//! crate `oxikube_settings` parses without depending on the app layer.

use std::fmt;
use std::sync::Arc;

use crate::ids::Gvr;

/// What an alias resolves to.
///
/// Cloning is cheap (reference counts only), so a lookup on every keystroke does not allocate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AliasTarget {
    /// A resource type: open its list.
    Gvr(Gvr),
    /// A command line for the jump bar to run, in k9s's `fred: pod fred app=blee` style: `name`
    /// is the first word (`pod`) and `args` the rest (`["fred", "app=blee"]`). The jump bar's
    /// parser resolves `name` once more through the alias table; a name that is itself a
    /// `Command` id (`pod::Delete`) dispatches on the command bus, so it is guarded like every
    /// other action.
    Command {
        /// The first word of the command line.
        name: Arc<str>,
        /// The remaining words, in order.
        args: Arc<[String]>,
    },
}

impl AliasTarget {
    /// A resource target.
    pub fn gvr(gvr: Gvr) -> Self {
        Self::Gvr(gvr)
    }

    /// A command target from its first word and the rest.
    pub fn command(name: impl Into<Arc<str>>, args: impl IntoIterator<Item = String>) -> Self {
        Self::Command {
            name: name.into(),
            args: args.into_iter().collect(),
        }
    }

    /// Whether `self` and `other` lead to the same place, ignoring the served version of a
    /// resource (`apps/v1/deployments` and `apps/v1beta1/deployments` are one target). Used to
    /// tell a real collision from two sources agreeing.
    pub fn same_destination(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Gvr(a), Self::Gvr(b)) => a.group == b.group && a.resource == b.resource,
            (a, b) => a == b,
        }
    }
}

impl fmt::Display for AliasTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gvr(gvr) => fmt::Display::fmt(gvr, f),
            Self::Command { name, args } => {
                f.write_str(name)?;
                for arg in args.iter() {
                    write!(f, " {arg}")?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_the_gvr_or_the_command_line() {
        let pods = AliasTarget::gvr(Gvr::new("", "v1", "pods"));
        assert_eq!(pods.to_string(), "v1/pods");
        let fred = AliasTarget::command("pod", ["fred".to_owned(), "app=blee".to_owned()]);
        assert_eq!(fred.to_string(), "pod fred app=blee");
        assert_eq!(AliasTarget::command("ctx", []).to_string(), "ctx");
    }

    #[test]
    fn the_version_does_not_make_two_targets_different() {
        let a = AliasTarget::gvr(Gvr::new("apps", "v1", "deployments"));
        let b = AliasTarget::gvr(Gvr::new("apps", "v1beta1", "deployments"));
        let other_group = AliasTarget::gvr(Gvr::new("extensions", "v1", "deployments"));
        assert!(a.same_destination(&b));
        assert!(!a.same_destination(&other_group));
        assert!(!a.same_destination(&AliasTarget::command("deployments", [])));
    }
}
