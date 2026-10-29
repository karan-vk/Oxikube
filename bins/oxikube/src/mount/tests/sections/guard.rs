//! The guard matrix (epic criterion 3): every mutating row action of every kind, dispatched on the
//! app's own bus the way its row sends it, is guarded as declared.
//!
//! For each one: the documented risk and tier are the declared ones (a dropped tier fails here),
//! the bus asks for that tier before anything runs, a confirmed dispatch leaves an audit record,
//! a read-only cluster refuses it for the user and for an agent before any request (audited as
//! denied), and it has an MCP tool stub. [`RISKS`] holds every mutating command the app declares:
//! a new one without an entry fails, so it cannot ship unguarded by this matrix.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::{DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::{Confirmation, preview_verb};
use oxikube_domain::Capabilities;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{COMMANDS, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_testkit::TestPorts;
use oxikube_testkit::sections::catalog;

use super::super::App;
use super::{served, target, ui};
use crate::app_state::AppState;

/// The risk of every mutating command (docs/CONTEXT.md, "Risk / ConfirmTier"): low and medium
/// confirm with one click, high and irreversible with the name typed.
const RISKS: &[(CommandId, Risk)] = &[
    // RoleBindings and ClusterRoleBindings (E17-S06): the binding dialog's save and the detail
    // panel's delete; a binding hands out privileges, so risk medium.
    (CommandId::BINDING_CREATE, Risk::Medium),
    (CommandId::BINDING_DELETE, Risk::Medium),
    (CommandId::BINDING_EDIT, Risk::Medium),
    (CommandId::CRONJOB_RESUME, Risk::Low),
    (CommandId::CRONJOB_SUSPEND, Risk::Low),
    (CommandId::CRONJOB_TRIGGER, Risk::Low),
    // Installing a Helm chart (E16-S09): the install tab's preview is its dry run; no row action.
    (CommandId::HELM_INSTALL, Risk::Low),
    // Rolling back and uninstalling (E16-S06): the Releases detail's history row and header.
    (CommandId::HELM_ROLLBACK, Risk::Medium),
    (CommandId::HELM_UNINSTALL, Risk::High),
    // Upgrading a Helm release (E16-S10): the upgrade tab's preview is its dry run; no row action.
    (CommandId::HELM_UPGRADE, Risk::Medium),
    (CommandId::INGRESS_CLASS_SET_DEFAULT, Risk::Low),
    (CommandId::JOB_RESUME, Risk::Low),
    (CommandId::JOB_SUSPEND, Risk::Low),
    (CommandId::NAMESPACE_CREATE, Risk::Low),
    (CommandId::NAMESPACE_DELETE, Risk::Irreversible),
    (CommandId::NODE_CORDON, Risk::Low),
    (CommandId::NODE_DRAIN, Risk::High),
    // A privileged pod on the node (E09-S09): a mutation, though it ends in a shell.
    (CommandId::NODE_SHELL, Risk::Medium),
    (CommandId::NODE_UNCORDON, Risk::Low),
    (CommandId::POD_DEBUG, Risk::Low),
    (CommandId::POD_DELETE, Risk::Medium),
    (CommandId::POD_EVICT, Risk::Medium),
    (CommandId::POD_FORCE_DELETE, Risk::High),
    (CommandId::POD_SANITIZE, Risk::Medium),
    // Uploading a file into a container (E20-S07): low for a new file, medium when it replaces one
    // (`Command::effective_risk`); the file browser's Upload button and drop target, not a row
    // action (it needs a local file).
    (CommandId::POD_UPLOAD_FILE, Risk::Low),
    (CommandId::PVC_RESIZE, Risk::Medium),
    // The manifest editor's apply (E10); a Secret's "Edit decoded" applies through it.
    (CommandId::RESOURCE_APPLY, Risk::Low),
    // Many objects as one plan (E20-S03): the apply-directory dialog (E20-S05) and agents; low is
    // the floor, the riskiest object raises it (`effective_risk`). No row action.
    (CommandId::RESOURCE_APPLY_PLAN, Risk::Low),
    // Copy to another cluster or namespace (E20-S08): classified as an apply on the target. The
    // declared floor is low; a Secret, RBAC, custom or cluster-scoped object raises it.
    (CommandId::RESOURCE_COPY_TO, Risk::Low),
    (CommandId::RESOURCE_DELETE, Risk::Medium),
    // Roles and ClusterRoles (E17-S05): the rule dialog's save and the detail panel's delete (which
    // counts the bindings first, so it is not a row action); an RBAC change is risk medium.
    (CommandId::ROLE_CREATE, Risk::Medium),
    (CommandId::ROLE_DELETE, Risk::Medium),
    (CommandId::ROLE_EDIT, Risk::Medium),
    // The bulk actions on a table's selection (E12-S19); a one-row selection is the row action.
    (CommandId::SELECTION_DELETE, Risk::Medium),
    (CommandId::SELECTION_LABEL, Risk::Medium),
    (CommandId::SELECTION_RESTART, Risk::Low),
    // A TokenRequest turned into a kubeconfig (E17-S09): the ServiceAccount panel's Kubeconfig
    // section and the row menu; the result stays in memory.
    (CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG, Risk::Low),
    (CommandId::STORAGECLASS_SET_DEFAULT, Risk::Low),
    (CommandId::WORKLOAD_RESTART, Risk::Low),
    (CommandId::WORKLOAD_ROLLBACK, Risk::Medium),
    (CommandId::WORKLOAD_SCALE, Risk::Low),
    (CommandId::WORKLOAD_SET_IMAGE, Risk::Low),
];

fn documented(id: CommandId) -> (Risk, ConfirmTier) {
    let risk = RISKS
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, risk)| *risk)
        .unwrap_or_else(|| panic!("{id} is a mutation with no entry in the guard matrix"));
    let tier = match risk {
        Risk::Low | Risk::Medium => ConfirmTier::Simple,
        Risk::High | Risk::Irreversible => ConfirmTier::TypeName,
    };
    (risk, tier)
}

/// Where the guard raises a tier for the object a command names (`delete_risk`): deleting a Node,
/// a PersistentVolume or a CustomResourceDefinition asks for the name typed.
const RAISED_BY_TARGET: &[(CommandId, &str, ConfirmTier)] = &[
    (CommandId::RESOURCE_DELETE, "Node", ConfirmTier::TypeName),
    (
        CommandId::RESOURCE_DELETE,
        "PersistentVolume",
        ConfirmTier::TypeName,
    ),
    // E19-S01's classify table: deleting a CRD deletes every object of its kind.
    (
        CommandId::RESOURCE_DELETE,
        "CustomResourceDefinition",
        ConfirmTier::TypeName,
    ),
];

/// The tier the guard asks for `id` on a `kind`: the documented one, or the raised one.
fn expected_tier(id: CommandId, kind: &str) -> ConfirmTier {
    RAISED_BY_TARGET
        .iter()
        .find(|(raised, on, _)| *raised == id && *on == kind)
        .map_or_else(|| documented(id).1, |(_, _, tier)| *tier)
}

#[test]
fn every_mutating_command_the_app_declares_has_a_matrix_entry() {
    let declared: Vec<CommandId> = COMMANDS
        .iter()
        .filter(|meta| meta.mutating)
        .map(|meta| meta.id)
        .collect();
    let listed: Vec<CommandId> = RISKS.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        declared, listed,
        "a mutation was added or removed: give it a row in RISKS (and a row action that \
         the matrix dispatches, or say why not)"
    );
    for meta in COMMANDS.iter().filter(|meta| meta.mutating) {
        let (risk, tier) = documented(meta.id);
        assert_eq!(meta.risk, Some(risk), "{}", meta.id);
        assert_eq!(meta.confirm, tier, "{}", meta.id);
    }
}

#[gpui::test]
fn every_kinds_mutations_are_guarded_audited_and_tools(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.connect_sections();
    let bus = app.bus();
    let rows = app.row_actions();
    let sessions = app
        .vcx
        .update(|_, cx| AppState::global(cx).services().sessions.clone());
    let cluster = TestPorts::cluster_id();
    let mut checked = 0;
    for case in catalog() {
        let target = target(case);
        let mutations = rows
            .actions_for(&served(case), Capabilities::all())
            .into_iter()
            .filter(|action| action.meta().mutating);
        for action in mutations {
            let id = action.meta().id;
            let what = format!("{id} on a {}", case.kind);
            let tier = expected_tier(id, case.kind);
            assert!(bus.tool(id).is_some(), "{what}: an MCP tool stub");
            let command = action.command_for(&target);

            // Asked first, at the declared tier; nothing ran. An irreversible command (a
            // PersistentVolume's delete) and a create, update, patch or apply (E19-S07) are
            // dry-run by the guard before they are asked, and that dry run is audited as one;
            // nothing else is. A row's bare command may be one the server refuses to dry-run (a
            // set image naming no container): then nothing is asked at all.
            let rehearsed = preview_verb(&command).is_some()
                || command
                    .effective_risk()
                    .is_some_and(Risk::requires_dry_run_diff);
            let audited = app.ports.state.audit_log().len();
            let request = match block_on(bus.dispatch(command.clone(), ui())) {
                Ok(Outcome::NeedsConfirmation(request)) => Some(request),
                Err(DispatchError::Handler(_)) if rehearsed => None,
                other => panic!("{what}: expected a confirmation request, got {other:?}"),
            };
            let asked = &app.ports.state.audit_log()[audited..];
            assert_eq!(asked.len(), usize::from(rehearsed), "{what}");
            assert!(asked.iter().all(|r| r.dry_run), "{what}");
            let audited = audited + asked.len();

            if let Some(request) = request {
                assert_eq!(request.tier, tier, "{what}");
                assert_eq!(request.command, id, "{what}");
                assert_eq!(request.preview.is_some(), rehearsed, "{what}");
                // Confirmed: it runs (the fakes may refuse it) and leaves its audit record.
                let confirmation = match tier {
                    ConfirmTier::TypeName => Confirmation::typed(
                        request.token,
                        request.expected_name.clone().expect("a name to type"),
                    ),
                    _ => Confirmation::simple(request.token),
                };
                let _ =
                    block_on(bus.dispatch(command.clone(), ui().with_confirmation(confirmation)));
                let records = app.ports.state.audit_log();
                assert!(
                    records[audited..].iter().any(
                        |r| r.cmd.as_ref() == id.as_str() && r.outcome != AuditOutcome::Denied
                    ),
                    "{what}: a confirmed dispatch is audited"
                );
            }

            // Read-only: refused for the user and for an agent, before any request, audited.
            sessions.set_read_only(&cluster, true).expect("a session");
            let writes = app.seeded_resources().mutating_calls().len();
            let audited = app.ports.state.audit_log().len();
            for initiator in [Initiator::Ui, Initiator::Agent] {
                let refused =
                    block_on(bus.dispatch(command.clone(), DispatchContext::new(initiator, "x")));
                assert!(
                    matches!(refused, Err(DispatchError::ReadOnly { .. })),
                    "{what} read-only for {initiator}: {refused:?}"
                );
            }
            assert_eq!(
                app.seeded_resources().mutating_calls().len(),
                writes,
                "{what}: nothing reached the cluster"
            );
            let denied = app.ports.state.audit_log()[audited..]
                .iter()
                .filter(|r| r.cmd.as_ref() == id.as_str() && r.outcome == AuditOutcome::Denied)
                .count();
            assert_eq!(denied, 2, "{what}: both refusals are audited");
            sessions.set_read_only(&cluster, false).expect("a session");
            checked += 1;
        }
    }
    // Every kind's delete, plus the kind actions of the workloads, nodes, namespaces, network
    // and storage stories.
    assert!(checked >= 60, "{checked} mutations checked");
}
