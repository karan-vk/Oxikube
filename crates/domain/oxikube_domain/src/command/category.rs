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
    /// Views, zoom, the palette, the jump bar, the key-binding help and the keymap file
    /// (`view::*`, `palette::*`, `jump::*`, `help::*`, `keymap::*`, `charts::*`).
    View,
    /// Clusters and kubeconfig sources (`cluster::*`, `kubeconfig::*`).
    Cluster,
    /// Namespace selection (`namespace::*`).
    Namespace,
    /// Resources, tables and custom resources (`resource::*`, `table::*`, `crd::*`,
    /// `ingressclass::*`, `secret::*`, `selection::*`).
    Resource,
    /// Workloads (`workload::*`, `cronjob::*`, `job::*`).
    Workload,
    /// Pods (`pod::*`).
    Pod,
    /// Nodes (`node::*`).
    Node,
    /// Services (`service::*`).
    Service,
    /// Storage (`pvc::*`, `storageclass::*`).
    Storage,
    /// The log viewer (`logs::*`).
    Logs,
    /// Terminals (`terminal::*`).
    Terminal,
    /// The manifest editor (`editor::*`).
    Editor,
    /// Kubernetes events (`events::*`).
    Events,
    /// A namespace no category claims (an extension's command, until it declares one).
    Other,
}

impl CommandCategory {
    /// Every category, in display order.
    pub const ALL: [CommandCategory; 16] = [
        CommandCategory::App,
        CommandCategory::Window,
        CommandCategory::View,
        CommandCategory::Cluster,
        CommandCategory::Namespace,
        CommandCategory::Resource,
        CommandCategory::Workload,
        CommandCategory::Pod,
        CommandCategory::Node,
        CommandCategory::Service,
        CommandCategory::Storage,
        CommandCategory::Logs,
        CommandCategory::Terminal,
        CommandCategory::Editor,
        CommandCategory::Events,
        CommandCategory::Other,
    ];

    /// The category of the command `id`, from its namespace.
    pub const fn of(id: CommandId) -> Self {
        const NAMESPACES: [(&str, CommandCategory); 29] = [
            ("app", CommandCategory::App),
            ("window", CommandCategory::Window),
            ("view", CommandCategory::View),
            ("palette", CommandCategory::View),
            ("help", CommandCategory::View),
            ("jump", CommandCategory::View),
            ("keymap", CommandCategory::View),
            ("charts", CommandCategory::View),
            ("cluster", CommandCategory::Cluster),
            ("kubeconfig", CommandCategory::Cluster),
            ("namespace", CommandCategory::Namespace),
            ("resource", CommandCategory::Resource),
            ("table", CommandCategory::Resource),
            ("crd", CommandCategory::Resource),
            ("ingressclass", CommandCategory::Resource),
            ("secret", CommandCategory::Resource),
            ("selection", CommandCategory::Resource),
            ("workload", CommandCategory::Workload),
            ("cronjob", CommandCategory::Workload),
            ("job", CommandCategory::Workload),
            ("pod", CommandCategory::Pod),
            ("node", CommandCategory::Node),
            ("service", CommandCategory::Service),
            ("pvc", CommandCategory::Storage),
            ("storageclass", CommandCategory::Storage),
            ("logs", CommandCategory::Logs),
            ("terminal", CommandCategory::Terminal),
            ("editor", CommandCategory::Editor),
            ("events", CommandCategory::Events),
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
            CommandCategory::Service => "Service",
            CommandCategory::Storage => "Storage",
            CommandCategory::Logs => "Logs",
            CommandCategory::Terminal => "Terminal",
            CommandCategory::Editor => "Editor",
            CommandCategory::Events => "Events",
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
    fn the_namespaces_added_by_parallel_stories_are_all_listed() {
        assert_eq!(
            CommandCategory::of(CommandId::new("charts::Open")),
            CommandCategory::View
        );
        assert_eq!(
            CommandCategory::of(CommandId::INGRESS_CLASS_SET_DEFAULT),
            CommandCategory::Resource
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
