//! [`CommandCategory`]: the group the palette and the help overlay list a command under.

use std::fmt;

use serde::Serialize;

use super::id::CommandId;

/// The group a command belongs to in the palette and the help overlay.
///
/// A category is derived from the command id's namespace ([`CommandCategory::of`]), so a new
/// command lands in the right group without a second declaration. The declaration order is
/// the display order (the app's `CommandBus::list` sorts by category, then title), and
/// the test `every_declared_command_has_a_named_category` fails when a new namespace has no
/// category yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandCategory {
    /// Application-level actions (`app::*`).
    App,
    /// Windows (`window::*`).
    Window,
    /// Views, zoom, the palette itself and the key-binding help (`view::*`, `palette::*`, `help::*`).
    View,
    /// Clusters and kubeconfig sources (`cluster::*`, `kubeconfig::*`).
    Cluster,
    /// Namespace selection (`namespace::*`).
    Namespace,
    /// Resources, tables and custom resources (`resource::*`, `table::*`, `crd::*`).
    Resource,
    /// Workloads (`workload::*`).
    Workload,
    /// Pods (`pod::*`).
    Pod,
    /// Nodes (`node::*`).
    Node,
    /// The log viewer (`logs::*`).
    Logs,
    /// Terminals (`terminal::*`).
    Terminal,
    /// A namespace no category claims (an extension's command, until it declares one).
    Other,
}

impl CommandCategory {
    /// Every category, in display order.
    pub const ALL: [CommandCategory; 12] = [
        CommandCategory::App,
        CommandCategory::Window,
        CommandCategory::View,
        CommandCategory::Cluster,
        CommandCategory::Namespace,
        CommandCategory::Resource,
        CommandCategory::Workload,
        CommandCategory::Pod,
        CommandCategory::Node,
        CommandCategory::Logs,
        CommandCategory::Terminal,
        CommandCategory::Other,
    ];

    /// The category of the command `id`, from its namespace.
    pub const fn of(id: CommandId) -> Self {
        const NAMESPACES: [(&str, CommandCategory); 16] = [
            ("app", CommandCategory::App),
            ("window", CommandCategory::Window),
            ("view", CommandCategory::View),
            ("palette", CommandCategory::View),
            ("help", CommandCategory::View),
            ("cluster", CommandCategory::Cluster),
            ("kubeconfig", CommandCategory::Cluster),
            ("namespace", CommandCategory::Namespace),
            ("resource", CommandCategory::Resource),
            ("table", CommandCategory::Resource),
            ("crd", CommandCategory::Resource),
            ("workload", CommandCategory::Workload),
            ("pod", CommandCategory::Pod),
            ("node", CommandCategory::Node),
            ("logs", CommandCategory::Logs),
            ("terminal", CommandCategory::Terminal),
        ];
        let mut i = 0;
        while i < NAMESPACES.len() {
            if in_namespace(id.as_str(), NAMESPACES[i].0) {
                return NAMESPACES[i].1;
            }
            i += 1;
        }
        CommandCategory::Other
    }

    /// The heading shown for the category.
    pub const fn label(self) -> &'static str {
        match self {
            CommandCategory::App => "Application",
            CommandCategory::Window => "Window",
            CommandCategory::View => "View",
            CommandCategory::Cluster => "Cluster",
            CommandCategory::Namespace => "Namespace",
            CommandCategory::Resource => "Resource",
            CommandCategory::Workload => "Workload",
            CommandCategory::Pod => "Pod",
            CommandCategory::Node => "Node",
            CommandCategory::Logs => "Logs",
            CommandCategory::Terminal => "Terminal",
            CommandCategory::Other => "Other",
        }
    }
}

impl fmt::Display for CommandCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Whether `id` (`namespace::Verb`) is in `namespace`; usable in `const` context.
const fn in_namespace(id: &str, namespace: &str) -> bool {
    let (id, ns) = (id.as_bytes(), namespace.as_bytes());
    if id.len() < ns.len() + 2 || id[ns.len()] != b':' {
        return false;
    }
    let mut i = 0;
    while i < ns.len() {
        if id[i] != ns[i] {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_follows_the_namespace_not_a_prefix() {
        assert_eq!(
            CommandCategory::of(CommandId::POD_DELETE),
            CommandCategory::Pod
        );
        assert_eq!(
            CommandCategory::of(CommandId::new("podium::Open")),
            CommandCategory::Other,
            "`pod` must not claim `podium`"
        );
        assert_eq!(
            CommandCategory::of(CommandId::new("a::B")),
            CommandCategory::Other,
            "`app` must not claim a shorter namespace"
        );
    }

    #[test]
    fn display_order_is_declaration_order() {
        let mut sorted = CommandCategory::ALL;
        sorted.sort();
        assert_eq!(sorted, CommandCategory::ALL);
        for category in CommandCategory::ALL {
            assert!(!category.label().is_empty());
        }
    }
}
