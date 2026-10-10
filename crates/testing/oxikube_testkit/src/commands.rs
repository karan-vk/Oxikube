//! Fixture commands for tests of the command palette, the help overlay and the keymap: a
//! deterministic mix of [`CommandMeta`]s in every category, view and selection shape, in any
//! number (the palette filters up to 2 000 inside a 5 ms budget).
//!
//! The metas are leaked (`'static`, like the declared registry's) so they can stand in for
//! `oxikube_domain::command::COMMANDS` entries. They are *not* declared commands: use them
//! with a `CommandIndex` built directly, not with `CommandRegistry` (which only registers
//! declared ids).
//!
//! ```
//! use oxikube_testkit::commands::fixture_commands;
//! let metas = fixture_commands(2_000);
//! assert_eq!(metas.len(), 2_000);
//! ```

use oxikube_domain::Capabilities;
use oxikube_domain::command::{CommandId, CommandMeta, CommandScope, SelectionKind, ViewContext};
use oxikube_domain::safety::Risk;

/// The namespaces (so categories) the fixtures cycle through.
const NAMESPACES: [&str; 8] = [
    "app", "cluster", "resource", "pod", "logs", "terminal", "node", "workload",
];

const TABLE_VIEWS: &[ViewContext] = &[ViewContext::Table, ViewContext::Detail];

/// `n` distinct fixture commands, ids `<namespace>::Fx<i>`, titles `Fixture <i> ...`. The
/// shape of command `i` depends only on `i % 8`, so any prefix is a valid mix:
///
/// | `i % 8` | shape |
/// |---|---|
/// | 0 | read, global, every view |
/// | 1 | read, cluster, log view |
/// | 2 | read, selection, one object, table and detail |
/// | 3 | mutation (low risk), many objects, table and detail |
/// | 4 | read, cluster, every view |
/// | 5 | exec-class, one core `Pod`, table and detail |
/// | 6 | read, resource kind, table and detail |
/// | 7 | read, global, terminal |
pub fn fixture_commands(n: usize) -> Vec<&'static CommandMeta> {
    (0..n).map(fixture_command).collect()
}

/// Fixture command number `i` (see [`fixture_commands`]).
pub fn fixture_command(i: usize) -> &'static CommandMeta {
    let id = CommandId::new(Box::leak(
        format!("{}::Fx{i}", NAMESPACES[i % NAMESPACES.len()]).into_boxed_str(),
    ));
    let title: &'static str = Box::leak(format!("Fixture {i:04} command").into_boxed_str());
    let none = Capabilities::empty();
    let meta = match i % 8 {
        0 => CommandMeta::read(id, title, CommandScope::Global, none),
        1 => {
            CommandMeta::read(id, title, CommandScope::Cluster, none).in_views(&[ViewContext::Logs])
        }
        2 => CommandMeta::read(id, title, CommandScope::Selection, none)
            .in_views(TABLE_VIEWS)
            .selecting(SelectionKind::One),
        3 => CommandMeta::mutation(id, title, CommandScope::Selection, Risk::Low, none)
            .in_views(TABLE_VIEWS)
            .selecting(SelectionKind::Many),
        4 => CommandMeta::read(id, title, CommandScope::Cluster, none),
        5 => CommandMeta::exec(id, title, CommandScope::Selection)
            .in_views(TABLE_VIEWS)
            .selecting(SelectionKind::core("Pod")),
        6 => CommandMeta::read(id, title, CommandScope::ResourceKind, none).in_views(TABLE_VIEWS),
        _ => CommandMeta::read(id, title, CommandScope::Global, none)
            .in_views(&[ViewContext::Terminal]),
    };
    Box::leak(Box::new(meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_distinct_and_mixed() {
        let metas = fixture_commands(64);
        let mut ids: Vec<_> = metas.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 64);
        assert!(metas.iter().any(|m| m.mutating));
        assert!(metas.iter().any(|m| m.exec));
        assert!(metas.iter().any(|m| !m.availability.views.is_empty()));
    }
}
