//! The dry-run preview (E19-S07): a create, update, patch or apply is dry-run by the guard
//! (`dryRun=All`) before it is confirmed, and the question carries the live object next to the
//! server's result, Secret values masked, with the server's warnings; a server that cannot dry-run
//! it gives a banner, never a block; a conflict on the commit offers a new preview.

use std::sync::Arc;

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::{ConflictDetails, ConflictReason, ErrorKind, OxiError, Resource};
use oxikube_ports::Patch;
use oxikube_testkit::{FakeWarningPort, ResourceCall};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::apply::Change;
use crate::command_bus::{CommandOutput, DispatchError, HandlerContext, Outcome};
use crate::guard::{
    Confirmation, ConfirmationError, DryRunPreview, DryRunResult, MAX_PREVIEW_OBJECTS,
};
use crate::testing::{Harness, ctx, declared, id};

/// Where the test handler finds the warning port of cluster `a`, once it is connected.
type WarningSlot = Arc<Mutex<Option<Arc<FakeWarningPort>>>>;

fn config_map(name: &str) -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "ConfigMap"), "shop", name)
}

fn secret_ref() -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "Secret"), "shop", "db")
}

/// `cronjob::Suspend` stands in for any previewed patch: the test handler merge-patches
/// `data.mode = blue` onto its target, once per object in `targets`, with the warnings the
/// server sends meanwhile.
fn patch_command(target: ResourceRef) -> Command {
    Command::CronJobSuspend { target }
}

/// A harness whose `cronjob::Suspend` merge-patches its target (and `extra` more ConfigMaps)
/// and makes the server warn once per patch.
fn harness(extra: usize, body: Value) -> (Harness, WarningSlot) {
    let slot: WarningSlot = Arc::default();
    let warnings = slot.clone();
    let h = Harness::with_extra(move |reg, _, _| {
        reg.register(
            declared(CommandId::CRONJOB_SUSPEND),
            move |cmd: Command, cx: HandlerContext| {
                let warnings = warnings.lock().clone();
                let body = body.clone();
                async move {
                    let mutation = cx.require_mutation()?;
                    let target = cmd.target().expect("a target").clone();
                    let mut targets = vec![target];
                    targets.extend((0..extra).map(|i| config_map(&format!("cm-{i}"))));
                    for target in &targets {
                        if let Some(port) = &warnings {
                            port.push_text("unknown field \"spec.foo\"");
                        }
                        mutation
                            .writer()
                            .patch(
                                &target.gvk,
                                target.namespace(),
                                &target.name,
                                &Patch::merge(body.clone()),
                                &mutation.write_options(),
                            )
                            .await?;
                    }
                    Ok(CommandOutput::message(if mutation.dry_run() {
                        "Dry run: it can be patched"
                    } else {
                        "Patched"
                    }))
                }
            },
        )
    });
    h.connect("a", false);
    *slot.lock() = Some(h.connector.ports_for(&id("a")).warnings);
    (h, slot)
}

fn live(target: &ResourceRef, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": target.gvk.kind.as_ref(),
        "metadata": {
            "name": target.name.as_ref(),
            "namespace": "shop",
            "resourceVersion": "7",
            "managedFields": [{"manager": "kubectl", "operation": "Update"}]
        },
        "data": data
    }))
    .expect("a valid object")
}

fn rehearsed(
    outcome: Result<Outcome, DispatchError>,
) -> (crate::ConfirmationRequest, DryRunResult) {
    let request = match outcome {
        Ok(Outcome::NeedsConfirmation(request)) => request,
        other => panic!("expected a confirmation request, got {other:?}"),
    };
    let Some(DryRunPreview::Rehearsed(result)) = request.preview.clone() else {
        panic!("expected the guard's dry run: {request:?}");
    };
    (request, result)
}

#[test]
fn a_patch_is_dry_run_before_it_is_asked_and_the_question_carries_the_diff_and_warnings() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "green"})));
    for initiator in Initiator::ALL {
        let (request, preview) = rehearsed(h.dispatch(
            patch_command(config_map("web")),
            ctx(initiator).with_cluster(id("a")),
        ));
        assert_eq!(request.initiator, initiator);
        assert_eq!(
            preview.message.as_deref(),
            Some("Dry run: it can be patched")
        );
        let [change] = preview.changes.as_slice() else {
            panic!("one object: {preview:?}");
        };
        assert_eq!(change.target, config_map("web"));
        assert_eq!(change.change, Change::Update);
        assert_eq!(change.live.as_deref().unwrap()["data"]["mode"], "green");
        assert_eq!(change.result["data"]["mode"], "blue");
        assert_eq!(preview.warnings, ["unknown field \"spec.foo\""]);
        assert_eq!(preview.omitted, 0);
    }
    // Every request asked for `dryRun=All`; nothing changed in the cluster.
    let writes = h.writes("a");
    assert_eq!(writes.len(), Initiator::ALL.len());
    assert!(writes.iter().all(ResourceCall::is_dry_run), "{writes:?}");
    assert_eq!(
        h.resources("a").objects()[0].get_str("/data/mode"),
        Some("green")
    );
    // Each dry run is an audited attempt marked as one.
    assert!(
        h.audit()
            .iter()
            .all(|r| r.dry_run && r.outcome == AuditOutcome::Succeeded)
    );
}

#[test]
fn confirming_sends_the_real_patch_and_does_not_reuse_the_dry_run() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "green"})));
    let command = patch_command(config_map("web"));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let (request, _) = rehearsed(h.dispatch(command.clone(), c.clone()));
    let out = h
        .dispatch(
            command,
            c.with_confirmation(Confirmation::simple(request.token)),
        )
        .expect("confirmed");
    assert_eq!(out.completed().unwrap().message.as_deref(), Some("Patched"));
    let dry: Vec<bool> = h.writes("a").iter().map(ResourceCall::is_dry_run).collect();
    assert_eq!(dry, [true, false], "the preview, then the real request");
    assert_eq!(
        h.resources("a").objects()[0].get_str("/data/mode"),
        Some("blue")
    );
}

#[test]
fn a_patch_of_an_object_that_does_not_exist_is_previewed_as_a_create() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .script()
        .patch
        .push_ok(live(&config_map("new"), json!({"mode": "blue"})));
    let (_, preview) = rehearsed(h.dispatch(
        patch_command(config_map("new")),
        ctx(Initiator::Agent).with_cluster(id("a")),
    ));
    assert_eq!(preview.changes[0].change, Change::Create);
    assert!(preview.changes[0].live.is_none());
}

#[test]
fn a_secret_is_masked_on_both_sides_of_the_preview() {
    let (h, _) = harness(0, json!({"data": {"password": "bmV3LXNlY3JldA=="}}));
    h.resources("a").insert(live(
        &secret_ref(),
        json!({"user": "YWRtaW4=", "password": "b2xkLXNlY3JldA=="}),
    ));
    let (request, preview) = rehearsed(h.dispatch(
        patch_command(secret_ref()),
        ctx(Initiator::Agent).with_cluster(id("a")),
    ));
    let change = &preview.changes[0];
    let (before, after) = (change.live.as_deref().unwrap(), &*change.result);
    // The keys show; the values are keyed fingerprints: the unchanged one compares equal, the
    // changed one does not, and no value (encoded or not) is anywhere in the request.
    assert_eq!(before["data"]["user"], after["data"]["user"]);
    assert_ne!(before["data"]["password"], after["data"]["password"]);
    let text = format!("{request:?}");
    for value in [
        "YWRtaW4=",
        "admin",
        "b2xkLXNlY3JldA==",
        "bmV3LXNlY3JldA==",
        "new-secret",
    ] {
        assert!(!text.contains(value), "{value} leaked: {text}");
    }
    assert!(
        after["data"]["password"]
            .as_str()
            .is_some_and(|v| v.starts_with("(hidden ")),
        "{after}"
    );
}

#[test]
fn a_delete_carries_no_preview_and_sends_nothing_before_the_answer() {
    let h = Harness::new();
    h.connect("a", false);
    let request = h.ask(crate::testing::pod_delete("a", "web"), ctx(Initiator::Ui));
    assert_eq!(request.preview, None, "a delete has no diff to show");
    assert!(h.writes("a").is_empty());
}

#[test]
fn a_server_that_cannot_dry_run_gives_a_banner_and_never_blocks() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "green"})));
    for refusal in [
        OxiError::unsupported("the server does not allow this method on the requested resource"),
        OxiError::validation("dryRun is not supported by the aggregated API metrics.example.com"),
        OxiError::validation("admission webhook \"audit.example.com\" does not support dry run"),
    ] {
        let reason = refusal.message().to_owned();
        h.resources("a").script().patch.push_err(refusal);
        let command = patch_command(config_map("web"));
        let c = ctx(Initiator::Ui).with_cluster(id("a"));
        let request = h.ask(command.clone(), c.clone());
        assert_eq!(
            request.preview,
            Some(DryRunPreview::NoPreviewAvailable { reason }),
            "the question is asked, with the server's words for the banner"
        );
        let out = h.dispatch(
            command,
            c.with_confirmation(Confirmation::simple(request.token)),
        );
        assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    }
    // Each refused dry run is audited as a failed dry run; each confirmed patch succeeded.
    let summary: Vec<(bool, AuditOutcome)> =
        h.audit().iter().map(|r| (r.dry_run, r.outcome)).collect();
    assert_eq!(
        summary,
        [
            (true, AuditOutcome::Failed),
            (false, AuditOutcome::Succeeded)
        ]
        .repeat(3)
    );
}

#[test]
fn a_dry_run_the_server_refuses_asks_nothing_and_leaves_no_token() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .script()
        .patch
        .push_err(OxiError::forbidden("configmaps \"web\" is forbidden"));
    let error = h
        .dispatch(
            patch_command(config_map("web")),
            ctx(Initiator::Agent).with_cluster(id("a")),
        )
        .unwrap_err();
    assert!(
        matches!(&error, DispatchError::Handler(e) if e.kind() == ErrorKind::Forbidden),
        "{error:?}"
    );
    assert_eq!(h.bus.guard().pending_confirmations(), 0);
    assert!(h.writes("a").iter().all(ResourceCall::is_dry_run));
}

#[test]
fn a_dry_run_of_many_objects_keeps_the_first_and_counts_the_rest() {
    let (h, _) = harness(MAX_PREVIEW_OBJECTS + 2, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "green"})));
    for i in 0..MAX_PREVIEW_OBJECTS + 2 {
        h.resources("a").insert(live(
            &config_map(&format!("cm-{i}")),
            json!({"mode": "green"}),
        ));
    }
    let (_, preview) = rehearsed(h.dispatch(
        patch_command(config_map("web")),
        ctx(Initiator::Ui).with_cluster(id("a")),
    ));
    assert_eq!(preview.changes.len(), MAX_PREVIEW_OBJECTS);
    assert_eq!(preview.omitted, 3);
    assert_eq!(preview.warnings.len(), 1, "each warning once");
}

#[test]
fn a_conflict_on_the_commit_offers_a_new_preview_against_the_live_object() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "green"})));
    let command = patch_command(config_map("web"));
    let c = ctx(Initiator::Ui).with_cluster(id("a"));
    let (request, _) = rehearsed(h.dispatch(command.clone(), c.clone()));

    // Someone changes the object between the preview and the commit: the server says 409.
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "red"})));
    h.resources("a").script().patch.push_err(
        OxiError::conflict(
            "the object has been modified; please apply your changes to the latest version",
        )
        .with_source(ConflictDetails {
            reason: ConflictReason::StaleVersion,
            causes: Vec::new(),
        }),
    );
    let error = h
        .dispatch(
            command.clone(),
            c.clone()
                .with_confirmation(Confirmation::simple(request.token)),
        )
        .unwrap_err();
    assert!(error.offers_new_preview(), "{error:?}");
    assert!(!DispatchError::Declined(CommandId::CRONJOB_SUSPEND).offers_new_preview());

    // The new preview is made against the object as it is now.
    let (_, preview) = rehearsed(h.dispatch(command, c));
    assert_eq!(
        preview.changes[0].live.as_deref().unwrap()["data"]["mode"],
        "red"
    );
    assert_eq!(
        h.resources("a").objects()[0].get_str("/data/mode"),
        Some("red"),
        "nothing was written"
    );
}

#[test]
fn the_confirmation_of_one_payload_never_approves_another() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.resources("a")
        .insert(live(&config_map("web"), json!({"mode": "green"})));
    let c = ctx(Initiator::Agent).with_cluster(id("a"));
    let (request, _) = rehearsed(h.dispatch(patch_command(config_map("web")), c.clone()));
    // The token was issued for `web`, previewed; it cannot confirm a patch of `other`.
    let error = h
        .dispatch(
            patch_command(config_map("other")),
            c.with_confirmation(Confirmation::simple(request.token)),
        )
        .unwrap_err();
    assert!(
        matches!(
            error,
            DispatchError::Confirmation(ConfirmationError::Mismatch)
        ),
        "{error:?}"
    );
    assert!(h.writes("a").iter().all(ResourceCall::is_dry_run));
}

#[test]
fn a_read_only_cluster_is_refused_before_the_dry_run() {
    let (h, _) = harness(0, json!({"data": {"mode": "blue"}}));
    h.manager.set_read_only(&id("a"), true).unwrap();
    for initiator in Initiator::ALL {
        let error = h
            .dispatch(
                patch_command(config_map("web")),
                ctx(initiator).with_cluster(id("a")),
            )
            .unwrap_err();
        assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    }
    assert!(
        h.resources("a").recorded_calls().is_empty(),
        "not even a dry run"
    );
}

#[test]
fn every_previewed_command_is_dry_run_by_the_guard() {
    for (id, previewed) in [
        (CommandId::RESOURCE_APPLY, true),
        (CommandId::WORKLOAD_SET_IMAGE, true),
        (CommandId::NAMESPACE_CREATE, true),
        (CommandId::ROLE_EDIT, true),
        (CommandId::POD_DELETE, false),
        (CommandId::WORKLOAD_SCALE, false),
        (CommandId::HELM_INSTALL, false),
    ] {
        let preview = crate::guard::preview_verb(&sample(id)).is_some();
        assert_eq!(preview, previewed, "{id}");
    }
}

/// A command of `id` with any payload (the preview table looks at the id alone).
fn sample(id: CommandId) -> Command {
    crate::testing_posture::sample(id, "a")
}
