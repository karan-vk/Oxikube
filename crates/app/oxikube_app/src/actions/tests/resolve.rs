//! Resolving the actions of a kind from the bus's registry.

use std::sync::Arc;

use oxikube_domain::Capabilities;
use oxikube_domain::command::{Command, CommandId, Propagation};
use oxikube_domain::kinds::Verb;
use oxikube_testkit::FakeClockPort;

use super::{ALL_VERBS, kind, namespaced};
use crate::actions::{
    ActionContext, ActionState, DisabledReason, DuplicateAction, KindFilter, RowActionRegistry,
    RowActionSpec, RowActions,
};
use crate::command_bus::{CommandBus, CommandRegistry};
use crate::guard::MutationGuard;
use crate::testing::Harness;

fn pods() -> oxikube_domain::kinds::ResourceKind {
    kind("", "Pod", "pods", &ALL_VERBS)
}

fn crd() -> oxikube_domain::kinds::ResourceKind {
    kind("example.io", "Widget", "widgets", &ALL_VERBS)
}

fn titles(actions: &[crate::actions::RowAction]) -> Vec<&'static str> {
    actions.iter().map(|a| a.label()).collect()
}

#[test]
fn delete_is_offered_for_a_pod_and_for_a_custom_resource() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let caps = Capabilities::MUTATE;
    assert_eq!(titles(&actions.actions_for(&pods(), caps)), ["Delete"]);
    assert_eq!(titles(&actions.actions_for(&crd(), caps)), ["Delete"]);
    let delete = actions.actions_for(&pods(), caps)[0];
    assert_eq!(delete.meta().id, CommandId::RESOURCE_DELETE);
    assert!(delete.meta().mutating);
}

#[test]
fn a_kind_the_server_cannot_delete_has_no_delete() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let read_only_kind = kind(
        "",
        "ComponentStatus",
        "componentstatuses",
        &[Verb::Get, Verb::List],
    );
    assert!(
        actions
            .actions_for(&read_only_kind, Capabilities::all())
            .is_empty()
    );
}

#[test]
fn actions_come_from_the_bus_not_from_the_registry_alone() {
    // A bus nobody registered resource::Delete on offers nothing, whatever the registry says.
    let h = Harness::new();
    let bare = CommandBus::new(
        CommandRegistry::new(),
        MutationGuard::new(
            h.manager.clone(),
            h.state,
            Arc::new(FakeClockPort::default()),
        ),
    );
    let actions = RowActions::from_bus(&bare, &RowActionRegistry::core());
    assert!(actions.all().is_empty());
    assert!(actions.actions_for(&pods(), Capabilities::all()).is_empty());
}

#[test]
fn a_session_without_mutate_does_not_see_delete() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    for caps in [
        Capabilities::empty(),
        Capabilities::LOGS | Capabilities::EXEC,
    ] {
        assert!(actions.actions_for(&pods(), caps).is_empty(), "{caps:?}");
        let ctx = ActionContext::new(caps);
        assert!(actions.resolve(&pods(), &ctx, 1).is_empty());
    }
}

#[test]
fn read_only_disables_delete_with_a_reason() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let writable = ActionContext::new(Capabilities::MUTATE);
    let resolved = actions.resolve(&pods(), &writable, 1);
    assert_eq!(resolved.len(), 1);
    assert!(resolved[0].state.is_enabled());

    let read_only = writable.read_only(true);
    let resolved = actions.resolve(&pods(), &read_only, 1);
    assert_eq!(resolved.len(), 1, "shown, not hidden");
    assert_eq!(
        resolved[0].state,
        ActionState::Disabled(DisabledReason::ReadOnly)
    );
    assert_eq!(
        resolved[0].state.reason().unwrap().to_string(),
        "This cluster is read-only"
    );
}

#[test]
fn a_session_reports_its_own_context() {
    let h = Harness::new();
    h.connect("a", true);
    let session = h.manager.get(&crate::testing::id("a")).unwrap();
    let ctx = ActionContext::of(&session);
    assert!(ctx.read_only);
    assert!(ctx.capabilities.contains(Capabilities::MUTATE));
}

#[test]
fn several_objects_keep_only_the_bulk_actions() {
    let h = Harness::new();
    let mut registry = RowActionRegistry::core();
    registry
        .register(
            RowActionSpec::new(CommandId::POD_DELETE, |target| Command::PodDelete {
                target: target.clone(),
                grace_period_seconds: None,
            })
            .label("Delete Pod")
            .kinds(KindFilter::Matching(|kind| &*kind.gvk.kind == "Pod"))
            .order(100),
        )
        .unwrap();
    let actions = RowActions::from_bus(&h.bus, &registry);
    let ctx = ActionContext::new(Capabilities::MUTATE);

    // Per-kind actions come from the same registry, in their own order, only for their kind.
    let one: Vec<_> = actions.resolve(&pods(), &ctx, 1);
    assert_eq!(
        one.iter().map(|r| r.action.label()).collect::<Vec<_>>(),
        ["Delete Pod", "Delete"]
    );
    let many: Vec<_> = actions.resolve(&pods(), &ctx, 5);
    assert_eq!(
        many.iter().map(|r| r.action.label()).collect::<Vec<_>>(),
        ["Delete"]
    );
    let custom: Vec<_> = actions.resolve(&crd(), &ctx, 1);
    assert_eq!(
        custom.iter().map(|r| r.action.label()).collect::<Vec<_>>(),
        ["Delete"]
    );
}

#[test]
fn an_action_builds_the_command_for_an_object() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let delete = actions.actions_for(&pods(), Capabilities::MUTATE)[0];
    let target = namespaced("a", "", "Pod", "web-0");
    assert_eq!(
        delete.command_for(&target),
        Command::ResourceDelete {
            target,
            propagation: Propagation::Background
        },
        "the default follows kubectl: background"
    );
}

#[test]
fn registering_a_command_twice_is_refused() {
    let mut registry = RowActionRegistry::core();
    let again = RowActionSpec::new(CommandId::RESOURCE_DELETE, |target| Command::ResourceOpen {
        target: target.clone(),
    });
    assert_eq!(
        registry.register(again),
        Err(DuplicateAction(CommandId::RESOURCE_DELETE))
    );
    assert_eq!(registry.specs().len(), 1);
}
