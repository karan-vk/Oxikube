//! `CommandBus::list(ctx)`, `all()` and `get(id)` (E11-S01): which commands can run where.

use std::collections::HashSet;

use oxikube_domain::Capabilities;
use oxikube_domain::command::{
    self, Command, CommandCategory, CommandId, CommandMeta, CommandScope, SelectionKind,
    ViewContext,
};
use oxikube_domain::ids::Gvk;
use oxikube_ports::ToolName;
use oxikube_testkit::commands::fixture_commands;
use proptest::prelude::*;

use crate::command_bus::{
    CommandContext, CommandIndex, CommandInfo, CommandOutput, CommandRegistry, DuplicateCommand,
    HandlerContext, Selection, Unavailable,
};
use crate::testing::Harness;

fn noop(
    _: Command,
    _: HandlerContext,
) -> futures::future::Ready<oxikube_domain::OxiResult<CommandOutput>> {
    futures::future::ready(Ok(CommandOutput::none()))
}

fn pod() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn node() -> Gvk {
    Gvk::new("", "v1", "Node")
}

fn deployment() -> Gvk {
    Gvk::new("apps", "v1", "Deployment")
}

/// A writable session in an active cluster with every capability.
fn session(view: ViewContext) -> CommandContext {
    let mut ctx = CommandContext::new(view).with_capabilities(Capabilities::all());
    ctx.cluster_active = true;
    ctx
}

fn ids(list: &[CommandInfo]) -> Vec<CommandId> {
    list.iter().map(CommandInfo::id).collect()
}

fn full_bus() -> crate::CommandBus {
    Harness::with_every_command().bus
}

#[test]
fn list_depends_on_the_view_context() {
    let bus = full_bus();
    let logs = ids(&bus.list(&session(ViewContext::Logs)));
    assert!(logs.contains(&CommandId::LOGS_FIND));
    assert!(
        logs.contains(&CommandId::APP_QUIT),
        "global commands run everywhere"
    );
    assert!(!logs.contains(&CommandId::TERMINAL_COPY));

    let terminal = ids(&bus.list(&session(ViewContext::Terminal)));
    assert!(terminal.contains(&CommandId::TERMINAL_COPY));
    assert!(!terminal.contains(&CommandId::LOGS_FIND));

    let table = ids(&bus.list(&session(ViewContext::Table)));
    assert!(table.contains(&CommandId::TABLE_FOCUS_FILTER));
    assert!(!table.contains(&CommandId::LOGS_FIND));
    assert!(!table.contains(&CommandId::TERMINAL_COPY));
}

#[test]
fn list_without_a_cluster_is_only_the_global_commands() {
    let bus = full_bus();
    let ctx = CommandContext::new(ViewContext::Workspace);
    let listed = bus.list(&ctx);
    assert!(!listed.is_empty());
    for info in &listed {
        assert_eq!(
            info.meta().scope,
            CommandScope::Global,
            "{} needs a cluster",
            info.id()
        );
    }
    let listed_ids = ids(&listed);
    assert!(listed_ids.contains(&CommandId::CLUSTER_CONNECT));
    assert!(!listed_ids.contains(&CommandId::NAMESPACE_SELECT));
    assert_eq!(
        bus.get(CommandId::NAMESPACE_SELECT).unwrap().check(&ctx),
        Err(Unavailable::NoCluster)
    );
}

#[test]
fn list_hides_mutations_on_a_read_only_session() {
    let bus = full_bus();
    let writable = session(ViewContext::Table).selecting(Selection::many(pod(), 2));
    let read_only = writable.clone().read_only(true);

    let w = ids(&bus.list(&writable));
    assert!(w.contains(&CommandId::RESOURCE_DELETE));
    assert!(w.contains(&CommandId::POD_DELETE));
    let r = ids(&bus.list(&read_only));
    assert!(!r.contains(&CommandId::RESOURCE_DELETE));
    assert!(!r.contains(&CommandId::POD_DELETE));
    assert!(
        r.contains(&CommandId::RESOURCE_COPY_NAME),
        "reads stay available read-only"
    );
    for info in bus.all() {
        if info.mutating() {
            assert!(!r.contains(&info.id()), "{}", info.id());
        }
    }
    assert_eq!(
        bus.get(CommandId::RESOURCE_DELETE)
            .unwrap()
            .check(&read_only),
        Err(Unavailable::ReadOnly),
        "show-all can say why"
    );
    // The read-only toggle is not a mutation: it stays, or nobody could lift the mode.
    let toggle = ids(&bus.list(&session(ViewContext::Workspace).read_only(true)));
    assert!(toggle.contains(&CommandId::CLUSTER_TOGGLE_READ_ONLY));
}

#[test]
fn exec_class_commands_follow_exec_in_read_only() {
    let bus = full_bus();
    let one_pod = session(ViewContext::Table)
        .selecting(Selection::one(pod()))
        .read_only(true);
    let shell = bus.get(CommandId::POD_SHELL).unwrap();
    assert_eq!(shell.check(&one_pod), Err(Unavailable::ExecReadOnly));
    let mut allowed = one_pod;
    allowed.exec_in_read_only = true;
    assert_eq!(shell.check(&allowed), Ok(()));
}

#[test]
fn list_depends_on_the_capabilities() {
    let bus = full_bus();
    let mut ctx = session(ViewContext::Logs);
    assert!(ids(&bus.list(&ctx)).contains(&CommandId::LOGS_COPY));
    ctx.capabilities = Capabilities::empty();
    assert!(!ids(&bus.list(&ctx)).contains(&CommandId::LOGS_COPY));
    assert_eq!(
        bus.get(CommandId::LOGS_COPY).unwrap().check(&ctx),
        Err(Unavailable::Capabilities(Capabilities::LOGS))
    );
}

#[test]
fn list_depends_on_the_selection_kind() {
    let bus = full_bus();
    let table = session(ViewContext::Table);
    let has = |ctx: &CommandContext, id: CommandId| ids(&bus.list(ctx)).contains(&id);

    // No selection: object actions are hidden, the table's own commands stay.
    assert!(!has(&table, CommandId::RESOURCE_VIEW_YAML));
    assert!(!has(&table, CommandId::RESOURCE_DELETE));
    assert!(!has(&table, CommandId::POD_SHELL));
    assert!(has(&table, CommandId::TABLE_FOCUS_FILTER));

    // One pod: one-object, many-capable and pod-only actions appear; node actions do not.
    let one = table.clone().selecting(Selection::one(pod()));
    assert!(has(&one, CommandId::RESOURCE_VIEW_YAML));
    assert!(has(&one, CommandId::RESOURCE_DELETE));
    assert!(has(&one, CommandId::POD_SHELL));
    assert!(!has(&one, CommandId::NODE_CORDON));

    // Two pods: one-object actions go, bulk actions stay.
    let many = table.clone().selecting(Selection::many(pod(), 2));
    assert!(!has(&many, CommandId::RESOURCE_VIEW_YAML));
    assert!(has(&many, CommandId::RESOURCE_DELETE));
    assert!(has(&many, CommandId::RESOURCE_COPY_NAME));
    assert!(has(&many, CommandId::POD_DELETE), "OfKind accepts several");

    // A node: node actions, not pod ones.
    let a_node = table.clone().selecting(Selection::one(node()));
    assert!(has(&a_node, CommandId::NODE_CORDON));
    assert!(!has(&a_node, CommandId::POD_SHELL));

    // A deployment: the workload actions.
    let a_deployment = table.selecting(Selection::one(deployment()));
    assert!(has(&a_deployment, CommandId::WORKLOAD_SCALE));
    assert!(!has(&a_deployment, CommandId::POD_DELETE));

    // Mixed kinds satisfy "many" but never "of kind".
    let mixed = session(ViewContext::Table).selecting(Selection::mixed(3));
    assert!(has(&mixed, CommandId::RESOURCE_DELETE));
    assert!(!has(&mixed, CommandId::POD_DELETE));
}

#[test]
fn selection_satisfies_each_requirement() {
    let none = Selection::none();
    let one = Selection::one(pod());
    let many = Selection::many(pod(), 3);
    let of_pod = SelectionKind::core("Pod");
    assert!(none.satisfies(SelectionKind::None));
    assert!(one.satisfies(SelectionKind::None));
    assert!(!none.satisfies(SelectionKind::One));
    assert!(one.satisfies(SelectionKind::One));
    assert!(!many.satisfies(SelectionKind::One));
    assert!(!none.satisfies(SelectionKind::Many));
    assert!(many.satisfies(SelectionKind::Many));
    assert!(many.satisfies(of_pod));
    assert!(!none.satisfies(of_pod));
    assert!(!Selection::one(deployment()).satisfies(of_pod));
    // A group mismatch is a different kind even with the same name.
    assert!(!Selection::one(Gvk::new("example.io", "v1", "Pod")).satisfies(of_pod));
    // The version is not part of a kind's identity.
    assert!(Selection::one(Gvk::new("", "v2", "Pod")).satisfies(of_pod));
}

#[test]
fn every_registered_command_is_in_all_with_a_tool_stub() {
    let bus = full_bus();
    assert_eq!(bus.all().len(), command::COMMANDS.len());
    for info in bus.all() {
        let meta = info.meta();
        assert_eq!(bus.get(info.id()).map(|i| i.id()), Some(info.id()));
        assert_eq!(info.title(), meta.title);
        assert_eq!(info.category(), CommandCategory::of(info.id()));
        assert_eq!(info.keymap_action(), info.id().as_str());
        assert_eq!(info.mutating(), meta.mutating);
        assert_eq!(info.confirm(), meta.confirm);
        assert_eq!(Some(info.owner()), bus.owner(info.id()));
        if meta.privileged {
            // ADR 0012: no tool for the commands that change the safety posture.
            assert!(
                !info.has_tool() && bus.tool(info.id()).is_none(),
                "{}",
                info.id()
            );
        } else {
            let tool = bus
                .tool(info.id())
                .unwrap_or_else(|| panic!("{} has no ToolDef", info.id()));
            assert!(info.has_tool());
            assert_eq!(tool.name, ToolName::new(&info.id().tool_name()).unwrap());
        }
    }
    assert!(bus.get(CommandId::new("nope::Missing")).is_none());
}

#[test]
fn duplicate_ids_are_rejected_by_the_registry_and_the_index() {
    let mut registry = CommandRegistry::new();
    let quit = command::lookup(CommandId::APP_QUIT).unwrap();
    registry.register(*quit, noop).unwrap();
    assert!(registry.register(*quit, noop).is_err());

    let err = CommandIndex::new([
        CommandInfo::new(quit, "a", true),
        CommandInfo::new(quit, "b", true),
    ])
    .unwrap_err();
    assert_eq!(err, DuplicateCommand(CommandId::APP_QUIT));
}

#[test]
fn order_is_category_then_title_whatever_the_registration_order() {
    let metas = fixture_commands(200);
    let info = |m: &&'static CommandMeta| CommandInfo::new(m, "fixture", true);
    let forward = CommandIndex::new(metas.iter().map(info)).unwrap();
    let backward = CommandIndex::new(metas.iter().rev().map(info)).unwrap();
    assert_eq!(ids(forward.all()), ids(backward.all()));
    let key = |i: &CommandInfo| (i.category(), i.title(), i.id());
    for pair in forward.all().windows(2) {
        assert!(key(&pair[0]) < key(&pair[1]), "{pair:?}");
    }

    let bus = full_bus();
    let ctx = session(ViewContext::Table).selecting(Selection::one(pod()));
    let listed = bus.list(&ctx);
    assert_eq!(ids(&listed), ids(&bus.list(&ctx)), "stable between calls");
    let categories: Vec<_> = listed.iter().map(CommandInfo::category).collect();
    assert!(categories.windows(2).all(|w| w[0] <= w[1]));
}

fn selection() -> impl Strategy<Value = Selection> {
    prop_oneof![
        Just(Selection::none()),
        Just(Selection::one(pod())),
        Just(Selection::one(node())),
        Just(Selection::many(pod(), 3)),
        Just(Selection::many(deployment(), 2)),
        Just(Selection::mixed(4)),
    ]
}

fn context() -> impl Strategy<Value = CommandContext> {
    (
        proptest::sample::select(ViewContext::ALL.to_vec()),
        any::<bool>(),
        0u32..128,
        any::<bool>(),
        any::<bool>(),
        selection(),
    )
        .prop_map(
            |(view, cluster_active, caps, read_only, exec_in_read_only, selection)| {
                CommandContext {
                    view,
                    cluster_active,
                    capabilities: Capabilities::from_bits_truncate(caps),
                    read_only,
                    exec_in_read_only,
                    selection,
                }
            },
        )
}

proptest! {
    /// `list` returns exactly the commands whose availability holds: never one that fails,
    /// never a runnable one missing.
    #[test]
    fn list_never_returns_an_unavailable_command(ctx in context()) {
        let bus = full_bus();
        let listed = bus.list(&ctx);
        for info in &listed {
            let meta = info.meta();
            prop_assert!(meta.availability.in_view(ctx.view));
            prop_assert!(meta.needs.satisfied_by(ctx.capabilities));
            prop_assert!(ctx.selection.satisfies(meta.availability.selection));
            prop_assert!(!(ctx.read_only && meta.availability.requires_writable));
            prop_assert!(ctx.cluster_active || meta.scope == CommandScope::Global);
        }
        let listed_ids: HashSet<_> = ids(&listed).into_iter().collect();
        for info in bus.all() {
            prop_assert_eq!(listed_ids.contains(&info.id()), info.check(&ctx).is_ok());
        }
    }
}

#[test]
fn listing_two_thousand_commands_is_well_inside_the_palette_budget() {
    let metas = fixture_commands(2_000);
    let index =
        CommandIndex::new(metas.iter().map(|m| CommandInfo::new(m, "fixture", true))).unwrap();
    let ctx = session(ViewContext::Table).selecting(Selection::one(pod()));
    let listed = index.list(&ctx);
    assert!(!listed.is_empty() && listed.len() < 2_000);
    let start = std::time::Instant::now();
    for _ in 0..50 {
        std::hint::black_box(index.list(std::hint::black_box(&ctx)));
    }
    let per_call = start.elapsed() / 50;
    // The palette filters up to 2 000 entries in 5 ms; listing them is a small part of it. The
    // bound is loose so a loaded CI machine does not flake; `benches/command_list` prints the
    // real numbers.
    assert!(
        per_call.as_millis() < 5,
        "list took {per_call:?} over 2000 commands"
    );
}
