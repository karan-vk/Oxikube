//! The RBAC soft gate on row actions: greyed with the verb named, read-only rows unaffected.

use std::sync::Arc;

use oxikube_domain::Capabilities;
use oxikube_domain::access::{AccessRule, AccessRules};
use oxikube_domain::command::CommandId;
use oxikube_testkit::{FakeAccessReviewPort, FakeClockPort};

use super::{ALL_VERBS, kind};
use crate::access::{AccessService, DEFAULT_TTL};
use crate::actions::{
    ActionAccess, ActionContext, ActionState, DisabledReason, RowActionRegistry, RowActions,
    required_access,
};
use crate::testing::Harness;

fn deployments() -> oxikube_domain::kinds::ResourceKind {
    kind("apps", "Deployment", "deployments", &ALL_VERBS)
}

/// A viewer in `dev`: reads everything, deletes nothing.
fn viewer_rules() -> AccessRules {
    AccessRules::none().with_rule(AccessRule::granting(
        &["get", "list", "watch"],
        &["apps"],
        &["deployments"],
        &[],
    ))
}

async fn access_in(rules: AccessRules, namespaces: &[&str]) -> ActionAccess {
    let port = Arc::new(FakeAccessReviewPort::new().with_rules(rules));
    let service = AccessService::new(port, Arc::new(FakeClockPort::default()), DEFAULT_TTL);
    for namespace in namespaces {
        service.rules(Some(namespace)).await.expect("rules");
    }
    ActionAccess::new(
        service,
        namespaces.iter().map(|ns| Some((*ns).to_owned())).collect(),
    )
}

fn delete_state(states: &[crate::actions::ResolvedAction]) -> ActionState {
    states
        .iter()
        .find(|resolved| resolved.action.meta().id == CommandId::RESOURCE_DELETE)
        .expect("delete is offered")
        .state
}

#[tokio::test]
async fn delete_is_greyed_with_its_verb_when_the_review_denies_it() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let ctx = ActionContext::new(Capabilities::all());
    let access = access_in(viewer_rules(), &["dev"]).await;

    let state = delete_state(&actions.resolve_with(&deployments(), &ctx, 1, &access));

    assert_eq!(
        state,
        ActionState::Disabled(DisabledReason::NoPermission { verb: "delete" })
    );
    let tooltip = state.reason().expect("a reason").to_string();
    assert!(tooltip.contains("no delete permission"), "{tooltip}");
}

#[tokio::test]
async fn delete_stays_enabled_when_the_review_allows_it_or_says_nothing() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let ctx = ActionContext::new(Capabilities::all());

    let admin = access_in(AccessRules::all_access(), &["dev"]).await;
    assert!(delete_state(&actions.resolve_with(&deployments(), &ctx, 1, &admin)).is_enabled());

    // Nothing reviewed yet (loading), and no gate at all: both fail open.
    assert!(
        delete_state(&actions.resolve_with(&deployments(), &ctx, 1, &ActionAccess::none()))
            .is_enabled()
    );
}

#[tokio::test]
async fn an_unreviewed_namespace_is_unknown_not_denied() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let ctx = ActionContext::new(Capabilities::all());
    let port = Arc::new(FakeAccessReviewPort::new().with_rules(viewer_rules()));
    let service = AccessService::new(port, Arc::new(FakeClockPort::default()), DEFAULT_TTL);
    let access = ActionAccess::new(service, vec![Some("never-reviewed".into())]);

    assert!(delete_state(&actions.resolve_with(&deployments(), &ctx, 1, &access)).is_enabled());
}

#[tokio::test]
async fn a_selection_across_namespaces_is_greyed_only_when_none_allows_it() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let ctx = ActionContext::new(Capabilities::all());
    let port = Arc::new(
        FakeAccessReviewPort::new()
            .with_namespace_rules("dev", viewer_rules())
            .with_namespace_rules("ops", AccessRules::all_access()),
    );
    let service = AccessService::new(port, Arc::new(FakeClockPort::default()), DEFAULT_TTL);
    for namespace in ["dev", "ops"] {
        service.rules(Some(namespace)).await.expect("rules");
    }

    let mixed = ActionAccess::new(
        service.clone(),
        vec![Some("dev".into()), Some("ops".into())],
    );
    assert!(delete_state(&actions.resolve_with(&deployments(), &ctx, 2, &mixed)).is_enabled());
    let dev_only = ActionAccess::new(service, vec![Some("dev".into())]);
    assert!(!delete_state(&actions.resolve_with(&deployments(), &ctx, 1, &dev_only)).is_enabled());
}

#[tokio::test]
async fn read_only_mode_is_reported_before_a_missing_permission() {
    let h = Harness::new();
    let actions = RowActions::from_bus(&h.bus, &RowActionRegistry::core());
    let ctx = ActionContext::new(Capabilities::all()).read_only(true);
    let access = access_in(viewer_rules(), &["dev"]).await;

    let state = delete_state(&actions.resolve_with(&deployments(), &ctx, 1, &access));

    assert_eq!(state, ActionState::Disabled(DisabledReason::ReadOnly));
}

#[test]
fn only_mutating_row_commands_map_to_a_verb() {
    assert_eq!(
        required_access(CommandId::RESOURCE_DELETE).map(|need| need.verb),
        Some("delete")
    );
    assert_eq!(
        required_access(CommandId::WORKLOAD_RESTART).map(|need| need.verb),
        Some("patch")
    );
    let scale = required_access(CommandId::WORKLOAD_SCALE).expect("scale");
    assert_eq!((scale.verb, scale.subresource), ("update", Some("scale")));
    let evict = required_access(CommandId::POD_EVICT).expect("evict");
    assert_eq!(
        (evict.verb, evict.subresource),
        ("create", Some("eviction"))
    );
    assert_eq!(
        required_access(CommandId::NODE_DRAIN).map(|need| need.verb),
        Some("patch")
    );
    for read in [
        CommandId::RESOURCE_OPEN,
        CommandId::RESOURCE_VIEW_YAML,
        CommandId::POD_VIEW_LOGS,
        CommandId::RESOURCE_COPY_NAME,
    ] {
        assert_eq!(required_access(read), None, "{read}");
    }
}
