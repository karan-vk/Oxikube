//! `role::Create`, `role::Edit` and `role::Delete` through the real guard over the testkit fakes.

mod links;

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ErrorKind, Resource};
use oxikube_testkit::ResourceCall;
use serde_json::{Value, json};

use super::{RoleManifest, register_commands};
use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::testing::{Harness, ctx, id};

const RBAC: &str = "rbac.authorization.k8s.io/v1";

pub(super) fn cluster_role(version: Option<&str>) -> Value {
    let mut metadata = json!({"name": "reader"});
    if let Some(version) = version {
        metadata["resourceVersion"] = json!(version);
    }
    json!({
        "apiVersion": RBAC, "kind": "ClusterRole", "metadata": metadata,
        "rules": [
            {"apiGroups": [""], "resources": ["pods"], "verbs": ["get", "list"]},
            {"nonResourceURLs": ["/healthz"], "verbs": ["get"]},
        ],
    })
}

fn role(version: Option<&str>) -> Value {
    let mut metadata = json!({"name": "reader", "namespace": "demo"});
    if let Some(version) = version {
        metadata["resourceVersion"] = json!(version);
    }
    json!({
        "apiVersion": RBAC, "kind": "Role", "metadata": metadata,
        "rules": [{"apiGroups": ["apps"], "resources": ["deployments"], "verbs": ["get"]}],
    })
}

fn setup(read_only: bool) -> Harness {
    let h = Harness::with_extra(|reg, _, _| register_commands(reg));
    h.connect("a", read_only);
    h
}

fn create(manifest: &Value) -> Command {
    Command::RoleCreate {
        cluster: id("a"),
        manifest: manifest.to_string(),
    }
}

fn edit(manifest: &Value) -> Command {
    Command::RoleEdit {
        cluster: id("a"),
        manifest: manifest.to_string(),
    }
}

fn delete(bindings: Option<u32>) -> Command {
    Command::RoleDelete {
        target: ResourceRef::namespaced(
            id("a"),
            Gvk::new("rbac.authorization.k8s.io", "v1", "Role"),
            "demo",
            "reader",
        ),
        bindings,
    }
}

fn completed(outcome: Result<Outcome, DispatchError>) -> Value {
    match outcome.expect("dispatched") {
        Outcome::Completed(output) => output.data.expect("data"),
        other @ Outcome::NeedsConfirmation(_) => panic!("not completed: {other:?}"),
    }
}

fn stored(h: &Harness, kind: &str, name: &str) -> Option<Resource> {
    h.resources("a")
        .objects()
        .into_iter()
        .find(|object| &*object.kind.kind == kind && object.name() == name)
}

#[test]
fn create_is_medium_risk_with_one_simple_confirmation_naming_the_role() {
    let h = setup(false);
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let asked = h.ask(create(&cluster_role(None)), c);
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert_eq!(asked.risk, Some(Risk::Medium));
    assert!(
        asked.summary.contains("ClusterRole reader") && asked.summary.contains("2 rules"),
        "{}",
        asked.summary
    );
    // Asking sends the guard's dry run only (E19-S07).
    let writes = h.writes("a");
    assert!(
        !writes.is_empty() && writes.iter().all(ResourceCall::is_dry_run),
        "asking writes nothing: {writes:?}"
    );
}

#[test]
fn create_dry_runs_then_creates_and_audits_a_rule_count_only() {
    let h = setup(false);
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let data = completed(h.confirm_and_run(create(&role(None)), c));
    assert_eq!(data["outcome"], "created");
    let writes = h.writes("a");
    let dry: Vec<bool> = writes.iter().map(ResourceCall::is_dry_run).collect();
    // The guard's dry run before it asks, the handler's own rehearsal, the create.
    assert_eq!(
        dry,
        [true, true, false],
        "the server rehearses it first: {writes:?}"
    );
    assert!(stored(&h, "Role", "reader").is_some());
    let audit = h.audit();
    let last = audit.last().expect("an audit record");
    assert_eq!(last.outcome, AuditOutcome::Succeeded);
    assert_eq!(last.detail.as_deref(), Some("rules=1"));
    assert_eq!(last.target.namespace(), Some("demo"));
    assert_eq!(&*last.target.gvk.kind, "Role");
    assert!(
        !format!("{last:?}").contains("deployments"),
        "never the rules"
    );
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_and_writes_nothing() {
    let h = setup(true);
    for initiator in [Initiator::Ui, Initiator::Agent] {
        let c = ctx(initiator).with_cluster(id("a"));
        let error = h.dispatch(create(&role(None)), c).unwrap_err();
        assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    }
    assert!(h.resources("a").mutating_calls().is_empty());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn edit_replaces_at_the_read_version_and_a_stale_one_is_a_conflict() {
    let h = setup(false);
    let live = Resource::from_json(cluster_role(Some("7"))).expect("object");
    h.resources("a").insert(live);
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let mut changed = cluster_role(Some("7"));
    changed["rules"] = json!([{"apiGroups": [""], "resources": ["pods"], "verbs": ["get"]}]);
    let data = completed(h.confirm_and_run(edit(&changed), c.clone()));
    assert_eq!(data["outcome"], "updated");
    let replaces: Vec<_> = h
        .writes("a")
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::Replace {
                object, options, ..
            } => Some((object, options.dry_run)),
            _ => None,
        })
        .collect();
    assert_eq!(
        replaces.len(),
        3,
        "the guard's dry run, the handler's, the replace"
    );
    assert_eq!(replaces[2].0["metadata"]["resourceVersion"], "7");
    assert!(!replaces[2].1);

    // The server answers a stale version with a conflict, which comes back as the error.
    h.resources("a")
        .script()
        .replace
        .push_err(oxikube_domain::OxiError::conflict(
            "the object has been modified",
        ));
    let error = h.confirm_and_run(edit(&changed), c).unwrap_err();
    assert!(
        matches!(&error, DispatchError::Handler(e) if e.kind() == ErrorKind::Conflict),
        "{error:?}"
    );
}

#[test]
fn edit_without_a_resource_version_is_refused() {
    let h = setup(false);
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    let error = h.confirm_and_run(edit(&cluster_role(None)), c).unwrap_err();
    assert!(
        matches!(&error, DispatchError::Handler(e) if e.kind() == ErrorKind::Validation),
        "{error:?}"
    );
    assert!(h.writes("a").is_empty());
}

#[test]
fn only_roles_and_cluster_roles_can_be_written() {
    let h = setup(false);
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    for bad in [
        json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "x", "namespace": "d"}}),
        json!({"apiVersion": RBAC, "kind": "RoleBinding", "metadata": {"name": "x", "namespace": "d"}}),
        json!({"apiVersion": RBAC, "kind": "Role", "metadata": {"name": "x"}}),
        json!({"apiVersion": RBAC, "kind": "ClusterRole", "metadata": {"name": "x", "namespace": "d"}}),
    ] {
        let result = h.confirm_and_run(create(&bad), c.clone());
        assert!(result.is_err(), "{bad} was written");
    }
    assert!(RoleManifest::parse("not json").is_err());
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_delete_warns_with_the_binding_count_and_deletes_after_a_dry_run() {
    let h = setup(false);
    h.resources("a")
        .insert(Resource::from_json(role(Some("3"))).expect("object"));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let asked = h.ask(delete(Some(2)), c.clone());
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert!(asked.summary.contains("2 bindings"), "{}", asked.summary);
    assert!(
        !h.ask(delete(Some(1)), c.clone())
            .summary
            .contains("2 bindings")
    );
    let data = completed(h.confirm_and_run(delete(Some(2)), c));
    assert_eq!(data["outcome"], "deleted");
    assert!(stored(&h, "Role", "reader").is_none());
    assert_eq!(
        h.audit().last().and_then(|r| r.detail.clone()).as_deref(),
        Some("bindings=2")
    );
}

#[test]
fn delete_refuses_a_kind_that_is_not_a_role() {
    let h = setup(false);
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    let command = Command::RoleDelete {
        target: ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "ConfigMap"), "demo", "x"),
        bindings: None,
    };
    assert!(h.confirm_and_run(command, c).is_err());
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_dry_run_dispatch_stops_after_the_server_dry_run() {
    let h = setup(false);
    let c = DispatchContext::new(Initiator::Agent, "agent")
        .with_cluster(id("a"))
        .with_dry_run(true);
    let data = completed(h.confirm_and_run(create(&cluster_role(None)), c));
    assert_eq!(data["outcome"], "dry_run");
    assert!(h.writes("a").iter().all(ResourceCall::is_dry_run));
    assert!(stored(&h, "ClusterRole", "reader").is_none());
}

#[test]
fn the_commands_have_tool_stubs() {
    for (id, tool) in [
        (CommandId::ROLE_CREATE, "k8s.role_create"),
        (CommandId::ROLE_EDIT, "k8s.role_edit"),
        (CommandId::ROLE_DELETE, "k8s.role_delete"),
        (CommandId::ROLE_OPEN_DIALOG, "k8s.role_open_dialog"),
    ] {
        assert_eq!(id.tool_name(), tool);
    }
}
