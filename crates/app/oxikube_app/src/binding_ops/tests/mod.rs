//! `binding::Create`, `binding::Edit` and `binding::Delete` through the real guard over the
//! testkit fakes.

mod choices;

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{ErrorKind, OxiError, Resource};
use oxikube_testkit::ResourceCall;
use serde_json::{Value, json};

use super::{BindingManifest, register_commands};
use crate::command_bus::{DispatchContext, DispatchError, Outcome};
use crate::testing::{Harness, ctx, id};

const RBAC: &str = "rbac.authorization.k8s.io/v1";
const GROUP: &str = "rbac.authorization.k8s.io";

pub(super) fn role_binding(role: &str, version: Option<&str>) -> Value {
    let mut metadata = json!({"name": "readers", "namespace": "demo"});
    if let Some(version) = version {
        metadata["resourceVersion"] = json!(version);
        metadata["uid"] = json!("u-1");
    }
    json!({
        "apiVersion": RBAC, "kind": "RoleBinding", "metadata": metadata,
        "roleRef": {"apiGroup": GROUP, "kind": "ClusterRole", "name": role},
        "subjects": [
            {"apiGroup": GROUP, "kind": "User", "name": "ana"},
            {"kind": "ServiceAccount", "name": "builder", "namespace": "demo"},
        ],
    })
}

fn cluster_binding(role: &str, subject: Value) -> Value {
    json!({
        "apiVersion": RBAC, "kind": "ClusterRoleBinding", "metadata": {"name": "admins"},
        "roleRef": {"apiGroup": GROUP, "kind": "ClusterRole", "name": role},
        "subjects": [subject],
    })
}

fn setup(read_only: bool) -> Harness {
    let h = Harness::with_extra(|reg, _, _| register_commands(reg));
    h.connect("a", read_only);
    h
}

fn create(manifest: &Value) -> Command {
    Command::BindingCreate {
        cluster: id("a"),
        manifest: manifest.to_string(),
    }
}

fn edit(manifest: &Value, previous: Option<&Value>) -> Command {
    Command::BindingEdit {
        cluster: id("a"),
        manifest: manifest.to_string(),
        previous: previous.map(Value::to_string),
    }
}

fn delete(kind: &str, namespace: Option<&str>, name: &str) -> Command {
    let gvk = Gvk::new(GROUP, "v1", kind);
    Command::BindingDelete {
        target: match namespace {
            Some(namespace) => ResourceRef::namespaced(id("a"), gvk, namespace, name),
            None => ResourceRef::cluster_scoped(id("a"), gvk, name),
        },
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

fn role_ref_of(object: &Resource) -> String {
    object.to_value()["roleRef"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn create_is_medium_risk_with_one_simple_confirmation_naming_binding_role_and_subjects() {
    let h = setup(false);
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let asked = h.ask(create(&role_binding("view", None)), c);
    assert_eq!(asked.tier, ConfirmTier::Simple);
    assert_eq!(asked.risk, Some(Risk::Medium));
    for part in ["RoleBinding demo/readers", "ClusterRole/view", "2 subjects"] {
        assert!(asked.summary.contains(part), "{part}: {}", asked.summary);
    }
    assert!(!asked.summary.contains("WARNING"));
    // Asking sends the guard's dry run only (E19-S07), and the question carries its result.
    let writes = h.writes("a");
    assert!(
        !writes.is_empty() && writes.iter().all(ResourceCall::is_dry_run),
        "asking writes nothing: {writes:?}"
    );
    let Some(crate::guard::DryRunPreview::Rehearsed(preview)) = &asked.preview else {
        panic!("a create is previewed: {asked:?}");
    };
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].change, crate::apply::Change::Create);
}

#[test]
fn a_grant_to_cluster_admin_or_to_everybody_is_called_out_in_the_confirmation() {
    let h = setup(false);
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let admin = cluster_binding("cluster-admin", json!({"kind": "User", "name": "ana"}));
    let asked = h.ask(create(&admin), c.clone());
    assert!(
        asked.summary.contains("WARNING: cluster-admin"),
        "{}",
        asked.summary
    );
    let everybody = cluster_binding(
        "view",
        json!({"kind": "Group", "name": "system:authenticated"}),
    );
    let asked = h.ask(create(&everybody), c);
    assert!(
        asked.summary.contains("WARNING: A subject here"),
        "{}",
        asked.summary
    );
}

#[test]
fn create_dry_runs_then_creates_and_audits_role_and_count_never_names() {
    let h = setup(false);
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let data = completed(h.confirm_and_run(create(&role_binding("view", Some("9"))), c));
    assert_eq!(data["outcome"], "created");
    let writes = h.writes("a");
    let dry: Vec<bool> = writes.iter().map(ResourceCall::is_dry_run).collect();
    // The guard's dry run before it asks, the handler's own rehearsal, the create.
    assert_eq!(dry, [true, true, false], "rehearsed first: {writes:?}");
    // The server-owned fields of a body that was read are not sent on a create.
    let ResourceCall::Create { object, .. } = &writes[2] else {
        panic!("a create: {writes:?}");
    };
    assert!(object["metadata"].get("resourceVersion").is_none());
    assert!(object["metadata"].get("uid").is_none());
    let audit = h.audit();
    let last = audit.last().expect("an audit record");
    assert_eq!(last.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        last.detail.as_deref(),
        Some("role=ClusterRole/view subjects=2")
    );
    assert_eq!(&*last.target.gvk.kind, "RoleBinding");
    assert!(!format!("{last:?}").contains("ana"), "never the subjects");
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_and_writes_nothing() {
    let h = setup(true);
    for initiator in [Initiator::Ui, Initiator::Agent] {
        let c = ctx(initiator).with_cluster(id("a"));
        let error = h
            .dispatch(create(&role_binding("view", None)), c)
            .unwrap_err();
        assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    }
    assert!(h.resources("a").mutating_calls().is_empty());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn wrong_subject_kinds_groups_and_role_refs_are_refused_before_anything_is_sent() {
    let h = setup(false);
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    let sa_without_ns = json!({"kind": "ServiceAccount", "name": "builder"});
    let sa_with_group =
        json!({"kind": "ServiceAccount", "name": "b", "namespace": "d", "apiGroup": GROUP});
    let user_wrong_group = json!({"kind": "User", "name": "ana", "apiGroup": "apps"});
    let robot = json!({"kind": "Robot", "name": "r2"});
    let nameless = json!({"kind": "User", "name": ""});
    let mut bad = vec![
        cluster_binding("view", sa_without_ns),
        cluster_binding("view", sa_with_group),
        cluster_binding("view", user_wrong_group),
        cluster_binding("view", robot),
        cluster_binding("view", nameless),
    ];
    let mut names_a_role = cluster_binding("view", json!({"kind": "User", "name": "ana"}));
    names_a_role["roleRef"]["kind"] = json!("Role");
    bad.push(names_a_role);
    let mut group = cluster_binding("view", json!({"kind": "User", "name": "ana"}));
    group["roleRef"]["apiGroup"] = json!("apps");
    bad.push(group);
    let mut namespaced = cluster_binding("view", json!({"kind": "User", "name": "ana"}));
    namespaced["metadata"]["namespace"] = json!("d");
    bad.push(namespaced);
    bad.push(json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "x", "namespace": "d"}}));
    bad.push(
        json!({"apiVersion": RBAC, "kind": "Role", "metadata": {"name": "x", "namespace": "d"}}),
    );
    for manifest in &bad {
        let result = h.confirm_and_run(create(manifest), c.clone());
        assert!(result.is_err(), "{manifest} was accepted");
    }
    assert!(BindingManifest::parse("not json").is_err());
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_service_account_subject_is_valid_with_a_namespace_and_no_api_group() {
    let manifest = BindingManifest::parse(&role_binding("view", None).to_string()).expect("valid");
    assert_eq!(manifest.subjects.len(), 2);
    assert_eq!(manifest.role_label(), "ClusterRole/view");
    assert_eq!(manifest.label(), "demo/readers");
}

#[test]
fn edit_replaces_in_place_at_the_read_version_and_a_stale_one_is_a_conflict() {
    let h = setup(false);
    h.resources("a")
        .insert(Resource::from_json(role_binding("view", Some("7"))).expect("object"));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let mut changed = role_binding("view", Some("7"));
    changed["subjects"] = json!([{"apiGroup": GROUP, "kind": "Group", "name": "team"}]);
    let data = completed(h.confirm_and_run(edit(&changed, None), c.clone()));
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

    // The object changed since it was read: the guard's dry run already says so, and nothing is
    // asked.
    h.resources("a")
        .script()
        .replace
        .push_err(OxiError::conflict("the object has been modified"));
    let error = h.confirm_and_run(edit(&changed, None), c).unwrap_err();
    assert!(
        matches!(&error, DispatchError::Handler(e) if e.kind() == ErrorKind::Conflict),
        "{error:?}"
    );
}

#[test]
fn an_in_place_edit_needs_the_resource_version() {
    let h = setup(false);
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    let error = h
        .confirm_and_run(edit(&role_binding("view", None), None), c)
        .unwrap_err();
    assert!(
        matches!(&error, DispatchError::Handler(e) if e.kind() == ErrorKind::Validation),
        "{error:?}"
    );
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_new_role_ref_is_a_delete_and_a_create_as_one_audited_intent() {
    let h = setup(false);
    let old = role_binding("view", Some("7"));
    h.resources("a")
        .insert(Resource::from_json(old.clone()).expect("object"));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let new = role_binding("edit", Some("7"));
    let asked = h.ask(edit(&new, Some(&old)), c.clone());
    assert!(
        asked
            .summary
            .contains("from ClusterRole/view to ClusterRole/edit")
            && asked.summary.contains("deleted and created again"),
        "{}",
        asked.summary
    );
    // The guard's dry run before it asked deleted nothing.
    assert!(h.writes("a").iter().all(ResourceCall::is_dry_run));
    let before = h.writes("a").len();
    let answer = crate::guard::Confirmation::simple(asked.token);
    let data = completed(h.dispatch(edit(&new, Some(&old)), c.with_confirmation(answer)));
    assert_eq!(data["outcome"], "recreated");
    let now = stored(&h, "RoleBinding", "readers").expect("the binding exists");
    assert_eq!(role_ref_of(&now), "edit");
    let audit = h.audit();
    let records: Vec<_> = audit
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Succeeded && !r.dry_run)
        .collect();
    assert_eq!(records.len(), 1, "one intent, one record: {audit:?}");
    assert_eq!(
        records[0].detail.as_deref(),
        Some("role=ClusterRole/edit subjects=2 recreate=delete+create")
    );
    let kinds: Vec<&str> = h.writes("a")[before..]
        .iter()
        .map(|call| match call {
            ResourceCall::Delete { .. } => "delete",
            ResourceCall::Create { .. } => "create",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["delete", "delete", "create", "create"]);
}

#[test]
fn if_the_new_binding_is_refused_the_old_one_is_put_back() {
    let h = setup(false);
    let old = role_binding("view", Some("7"));
    h.resources("a")
        .insert(Resource::from_json(old.clone()).expect("object"));
    h.resources("a")
        .script()
        .create
        .push_err(OxiError::validation("roleRef.name: Not found: \"nope\""));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let error = h
        .confirm_and_run(edit(&role_binding("nope", Some("7")), Some(&old)), c)
        .unwrap_err();
    let DispatchError::Handler(error) = &error else {
        panic!("{error:?}");
    };
    assert!(
        error
            .message()
            .contains("the old binding (ClusterRole/view) was put back"),
        "{}",
        error.message()
    );
    let now = stored(&h, "RoleBinding", "readers").expect("restored");
    assert_eq!(role_ref_of(&now), "view");
}

#[test]
fn a_recreate_keeps_kind_namespace_and_name() {
    let h = setup(false);
    let old = role_binding("view", Some("7"));
    let mut other = role_binding("edit", Some("7"));
    other["metadata"]["name"] = json!("someone-else");
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    assert!(h.confirm_and_run(edit(&other, Some(&old)), c).is_err());
    assert!(h.writes("a").is_empty());
}

#[test]
fn delete_asks_once_removes_the_binding_and_refuses_other_kinds() {
    let h = setup(false);
    h.resources("a")
        .insert(Resource::from_json(role_binding("view", Some("3"))).expect("object"));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let asked = h.ask(delete("RoleBinding", Some("demo"), "readers"), c.clone());
    assert_eq!(asked.tier, ConfirmTier::Simple);
    let data =
        completed(h.confirm_and_run(delete("RoleBinding", Some("demo"), "readers"), c.clone()));
    assert_eq!(data["outcome"], "deleted");
    assert!(stored(&h, "RoleBinding", "readers").is_none());

    let wrong = Command::BindingDelete {
        target: ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "ConfigMap"), "demo", "x"),
    };
    assert!(h.confirm_and_run(wrong, c).is_err());
}

#[test]
fn a_dry_run_dispatch_stops_after_the_server_dry_run() {
    let h = setup(false);
    let c = DispatchContext::new(Initiator::Agent, "agent")
        .with_cluster(id("a"))
        .with_dry_run(true);
    let data = completed(h.confirm_and_run(create(&role_binding("view", None)), c));
    assert_eq!(data["outcome"], "dry_run");
    assert!(h.writes("a").iter().all(ResourceCall::is_dry_run));
    assert!(stored(&h, "RoleBinding", "readers").is_none());
}

#[test]
fn the_commands_have_tool_stubs() {
    for (id, tool) in [
        (CommandId::BINDING_CREATE, "k8s.binding_create"),
        (CommandId::BINDING_EDIT, "k8s.binding_edit"),
        (CommandId::BINDING_DELETE, "k8s.binding_delete"),
        (CommandId::BINDING_OPEN_DIALOG, "k8s.binding_open_dialog"),
    ] {
        assert_eq!(id.tool_name(), tool);
    }
}
