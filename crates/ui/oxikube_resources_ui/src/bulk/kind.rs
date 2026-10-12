//! [`BulkKind`]: which bulk action a table action stands for.

use oxikube_domain::command::CommandId;
use oxikube_domain::kinds::ResourceKind;

/// A bulk action on a selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BulkKind {
    /// `selection::Delete`.
    Delete,
    /// `selection::Restart`.
    Restart,
    /// `selection::Label` (labels or annotations).
    Label,
}

impl BulkKind {
    /// The bulk action the table action `command` is for `selected` rows of `kind`, if it is one.
    ///
    /// The single-object delete and restart commands become the bulk action once several rows are
    /// selected; the `selection::*` commands always are. A Namespace keeps its own delete flow
    /// (`namespace::Delete` carries what the namespace holds into the audit record).
    pub fn of_action(command: CommandId, selected: usize, kind: &ResourceKind) -> Option<Self> {
        match command {
            CommandId::SELECTION_DELETE if !kind.gvk.is_namespace() => Some(Self::Delete),
            CommandId::RESOURCE_DELETE | CommandId::POD_DELETE
                if selected > 1 && !kind.gvk.is_namespace() =>
            {
                Some(Self::Delete)
            }
            CommandId::SELECTION_RESTART => Some(Self::Restart),
            CommandId::WORKLOAD_RESTART if selected > 1 => Some(Self::Restart),
            CommandId::SELECTION_LABEL => Some(Self::Label),
            _ => None,
        }
    }

    /// The row action whose rights and read-only state the bulk action follows: the single-object
    /// action the menu lists for it.
    pub fn row_action(self, kind: &ResourceKind) -> CommandId {
        match self {
            Self::Delete if kind.gvk.is_pod() => CommandId::POD_DELETE,
            Self::Delete => CommandId::RESOURCE_DELETE,
            Self::Restart => CommandId::WORKLOAD_RESTART,
            Self::Label => CommandId::SELECTION_LABEL,
        }
    }

    /// The command the dialog runs.
    pub fn command(self) -> CommandId {
        match self {
            Self::Delete => CommandId::SELECTION_DELETE,
            Self::Restart => CommandId::SELECTION_RESTART,
            Self::Label => CommandId::SELECTION_LABEL,
        }
    }

    /// The verb on the dialog's button: `Delete`, `Restart`, `Apply`.
    pub fn verb(self) -> &'static str {
        match self {
            Self::Delete => "Delete",
            Self::Restart => "Restart",
            Self::Label => "Apply",
        }
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::Gvk;
    use oxikube_domain::kinds::VerbSet;

    use super::*;

    fn kind(group: &str, name: &str) -> ResourceKind {
        ResourceKind {
            gvk: Gvk::new(group, "v1", name),
            preferred: true,
            plural: name.to_lowercase() + "s",
            singular: name.to_lowercase(),
            short_names: Vec::new(),
            categories: Vec::new(),
            verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
            namespaced: true,
        }
    }

    #[test]
    fn several_rows_turn_the_single_object_actions_into_the_bulk_ones() {
        let pods = kind("", "Pod");
        assert_eq!(
            BulkKind::of_action(CommandId::POD_DELETE, 3, &pods),
            Some(BulkKind::Delete)
        );
        assert_eq!(
            BulkKind::of_action(CommandId::RESOURCE_DELETE, 2, &pods),
            Some(BulkKind::Delete)
        );
        assert_eq!(BulkKind::of_action(CommandId::POD_DELETE, 1, &pods), None);
        let deployments = kind("apps", "Deployment");
        assert_eq!(
            BulkKind::of_action(CommandId::WORKLOAD_RESTART, 2, &deployments),
            Some(BulkKind::Restart)
        );
        assert_eq!(
            BulkKind::of_action(CommandId::WORKLOAD_RESTART, 1, &deployments),
            None
        );
        assert_eq!(
            BulkKind::of_action(CommandId::SELECTION_LABEL, 1, &pods),
            Some(BulkKind::Label)
        );
        assert_eq!(
            BulkKind::of_action(CommandId::SELECTION_RESTART, 1, &deployments),
            Some(BulkKind::Restart)
        );
        assert_eq!(BulkKind::of_action(CommandId::NODE_CORDON, 5, &pods), None);
    }

    #[test]
    fn a_namespace_keeps_its_own_delete() {
        let namespaces = kind("", "Namespace");
        for command in [
            CommandId::RESOURCE_DELETE,
            CommandId::NAMESPACE_DELETE,
            CommandId::SELECTION_DELETE,
        ] {
            assert_eq!(
                BulkKind::of_action(command, 3, &namespaces),
                None,
                "{command}"
            );
        }
        // But a namespace can be labelled in bulk.
        assert_eq!(
            BulkKind::of_action(CommandId::SELECTION_LABEL, 3, &namespaces),
            Some(BulkKind::Label)
        );
    }
}
