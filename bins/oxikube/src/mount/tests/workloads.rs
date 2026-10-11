//! Trigger, suspend and resume on the real init path (E12-S08): the Workloads section's CronJobs
//! and Jobs entries open their tables, `t` and `s` reach the app's bus and guard, ask to confirm,
//! dry-run the request, write, audit (the trigger's record names the Job it made) and offer to
//! open that Job; a read-only cluster refuses every one for every initiator; the five commands
//! are on the bus with tool stubs.

use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::Resource;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::ResourceCall;
use oxikube_testkit::TestPorts;
use oxikube_workspace::modal::DialogModal;
use serde_json::json;

use super::App;
use crate::app_state::AppState;

fn served(kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("batch", "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: vec!["all".into()],
        verbs: VerbSet::from_names(["get", "list", "watch", "delete", "patch", "create"]),
        namespaced: true,
    }
}

fn object(kind: &str, name: &str, suspend: bool, status: serde_json::Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "batch/v1", "kind": kind,
        "metadata": {"name": name, "namespace": "default", "uid": format!("{name}-uid"),
                     "resourceVersion": "1", "creationTimestamp": "2026-10-01T00:00:00Z"},
        "spec": {
            "suspend": suspend, "schedule": "0 3 * * *",
            "jobTemplate": {"spec": {"template": {"spec": {
                "restartPolicy": "Never",
                "containers": [{"name": "run", "image": "busybox"}],
            }}}},
        },
        "status": status,
    }))
    .unwrap()
}

fn cronjob_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        TestPorts::cluster_id(),
        Gvk::new("batch", "v1", "CronJob"),
        "default",
        name,
    )
}

impl App {
    /// The `entry` table (a `workloads/…` sidebar entry) of the connected cluster, the first row
    /// under the cursor.
    fn open_batch_table(
        &mut self,
        entry: &'static str,
        objects: impl IntoIterator<Item = Resource>,
    ) {
        self.serve([served("CronJob", "cronjobs"), served("Job", "jobs")]);
        let ports = self.ports.connector.ports_for(&TestPorts::cluster_id());
        for object in objects {
            ports.resources.insert(object);
        }
        self.press("enter");
        self.tick();
        self.click(entry);
        self.tick();
        let ws = self.tab_workspace();
        let table = self
            .vcx
            .update(|_, cx| ws.read(cx).items_of_type::<ResourceTable>().remove(0));
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
        self.press("down");
    }

    fn dialog_text(&mut self) -> Option<(String, String)> {
        let tab = self.tab_workspace();
        self.vcx.update(|_, cx| {
            let dialog = tab
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()?;
            let dialog = dialog.read(cx);
            Some((
                dialog.title().to_string(),
                dialog.message_text()?.to_string(),
            ))
        })
    }

    fn writes(&self) -> Vec<ResourceCall> {
        self.ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources
            .mutating_calls()
    }
}

#[gpui::test]
fn the_five_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, tool) in [
        (CommandId::CRONJOB_TRIGGER, "k8s.cronjob_trigger"),
        (CommandId::CRONJOB_SUSPEND, "k8s.cronjob_suspend"),
        (CommandId::CRONJOB_RESUME, "k8s.cronjob_resume"),
        (CommandId::JOB_SUSPEND, "k8s.job_suspend"),
        (CommandId::JOB_RESUME, "k8s.job_resume"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube_app::workloads"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
    let offered: Vec<_> = crate::mount::logs::row_actions()
        .specs()
        .iter()
        .map(|spec| spec.command)
        .collect();
    for id in [
        CommandId::CRONJOB_TRIGGER,
        CommandId::CRONJOB_SUSPEND,
        CommandId::CRONJOB_RESUME,
        CommandId::JOB_SUSPEND,
        CommandId::JOB_RESUME,
    ] {
        assert!(offered.contains(&id), "{id} is a row action");
    }
}

#[gpui::test]
fn the_cronjobs_entry_of_the_workloads_section_opens_the_cronjob_table(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_batch_table(
        "sidebar-entry-workloads/cronjobs",
        [object("CronJob", "nightly", false, json!({}))],
    );
    assert_eq!(app.tables(), ["CronJob"]);
}

#[gpui::test]
fn t_confirms_then_creates_the_job_audits_it_and_offers_to_open_it(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_batch_table(
        "sidebar-entry-workloads/cronjobs",
        [object("CronJob", "nightly", false, json!({}))],
    );
    app.press("t");
    app.tick();

    let (title, message) = app.dialog_text().expect("the guard asks first");
    assert_eq!(title, "Run the CronJob now?");
    assert!(message.contains("default/nightly"), "{message}");
    assert!(message.contains("job template"), "{message}");
    assert!(app.writes().is_empty(), "nothing is sent before the answer");

    app.press("enter");
    app.tick();
    app.tick();

    // The server rehearsed the Job, then it was created: the generated name, the owner, the
    // instantiate annotation.
    let writes = app.writes();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert!(writes[0].is_dry_run() && !writes[1].is_dry_run());
    let ResourceCall::Create { object, .. } = &writes[1] else {
        panic!("a create: {writes:?}");
    };
    assert_eq!(object["metadata"]["generateName"], "nightly-manual-");
    assert_eq!(object["metadata"]["ownerReferences"][0]["name"], "nightly");
    assert_eq!(
        object["metadata"]["annotations"]["cronjob.kubernetes.io/instantiate"],
        "manual"
    );

    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "cronjob::Trigger");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(audit[0].detail.as_deref(), Some("job=nightly-manual-x7k2p"));

    let toasts = app.toasts();
    assert!(
        toasts
            .iter()
            .any(|t| t.contains("created Job default/nightly-manual-x7k2p")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn s_suspends_a_cronjob_after_a_confirmation_and_the_palette_resumes_it(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_batch_table(
        "sidebar-entry-workloads/cronjobs",
        [object("CronJob", "nightly", false, json!({}))],
    );
    app.press("s");
    app.tick();
    let (title, message) = app.dialog_text().expect("a confirmation");
    assert_eq!(title, "Suspend it?");
    assert!(message.contains("no Jobs until it is resumed"), "{message}");
    app.press("enter");
    app.tick();
    app.tick();
    let writes = app.writes();
    let ResourceCall::Patch { patch, .. } = writes.last().expect("a patch") else {
        panic!("a patch: {writes:?}");
    };
    assert_eq!(patch.body, json!({"spec": {"suspend": true}}));
    assert!(writes[0].is_dry_run());

    // Resume is the same guarded door for the palette and an agent (the fake cluster streams no
    // watch event for the patch, so the row still reads as running and `s` would suspend again).
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    let resume = Command::CronJobResume {
        target: cronjob_ref("nightly"),
    };
    let ctx = DispatchContext::new(Initiator::Command, "palette");
    let request = futures::executor::block_on(bus.dispatch(resume.clone(), ctx.clone()))
        .expect("asks")
        .confirmation()
        .expect("a confirmation request");
    let done = futures::executor::block_on(bus.dispatch(
        resume,
        ctx.with_confirmation(oxikube_app::Confirmation::simple(request.token)),
    ))
    .expect("resumes");
    assert_eq!(
        done.completed().unwrap().message.as_deref(),
        Some("Resumed CronJob default/nightly")
    );
    let audit = app.ports.state.audit_log();
    let ids: Vec<_> = audit.iter().map(|r| r.cmd.to_string()).collect();
    assert_eq!(ids, ["cronjob::Suspend", "cronjob::Resume"]);
    assert_eq!(audit[1].initiator, Initiator::Command);
}

#[gpui::test]
fn declining_the_confirmation_changes_nothing_and_is_audited(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_batch_table(
        "sidebar-entry-workloads/cronjobs",
        [object("CronJob", "nightly", false, json!({}))],
    );
    app.press("t");
    app.tick();
    assert!(app.dialog_text().is_some());
    app.press("escape");
    app.tick();
    assert!(app.writes().is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, AuditOutcome::Cancelled);
}

#[gpui::test]
fn s_on_a_job_suspends_it_and_a_finished_job_is_not_resumed(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_batch_table(
        "sidebar-entry-workloads/jobs",
        [
            object("Job", "a-running", false, json!({"active": 1})),
            object(
                "Job",
                "b-done",
                true,
                json!({"conditions": [{"type": "Complete", "status": "True"}]}),
            ),
        ],
    );
    app.press("s");
    app.tick();
    let (title, message) = app.dialog_text().expect("a confirmation");
    assert_eq!(title, "Suspend it?");
    assert!(message.contains("running pods are deleted"), "{message}");
    app.press("enter");
    app.tick();
    app.tick();
    let writes = app.writes();
    assert!(
        matches!(writes.last(), Some(ResourceCall::Patch { name, .. }) if name == "a-running"),
        "{writes:?}"
    );

    // The finished Job: `s` resumes it (it reads as suspended), the handler refuses, nothing is
    // sent and the refusal is audited as a failure.
    app.press("down");
    let before = app.writes().len();
    app.press("s");
    app.tick();
    app.press("enter");
    app.tick();
    app.tick();
    assert_eq!(app.writes().len(), before, "no request for a finished Job");
    let toasts = app.toasts();
    assert!(
        toasts
            .iter()
            .any(|t| t.contains("has finished and cannot be resumed")),
        "{toasts:?}"
    );
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.last().map(|r| r.outcome), Some(AuditOutcome::Failed));
}

#[gpui::test]
fn a_read_only_cluster_refuses_every_command_whoever_asks(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.open_batch_table(
        "sidebar-entry-workloads/cronjobs",
        [object("CronJob", "nightly", false, json!({}))],
    );
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&TestPorts::cluster_id(), true)
        .expect("the session is open");

    // The keys say why and ask nothing.
    for key in ["t", "s"] {
        app.press(key);
        app.tick();
        assert!(app.dialog_text().is_none(), "{key}");
    }
    app.expect_toast("This cluster is read-only");

    let bus = state.command_bus().expect("the bus");
    for command in [
        Command::CronJobTrigger {
            target: cronjob_ref("nightly"),
        },
        Command::CronJobSuspend {
            target: cronjob_ref("nightly"),
        },
        Command::CronJobResume {
            target: cronjob_ref("nightly"),
        },
    ] {
        for initiator in [
            Initiator::Agent,
            Initiator::Command,
            Initiator::Ui,
            Initiator::Plugin,
        ] {
            let outcome = futures::executor::block_on(
                bus.dispatch(command.clone(), DispatchContext::new(initiator, "someone")),
            );
            assert!(
                matches!(outcome, Err(oxikube_app::DispatchError::ReadOnly { .. })),
                "{initiator} {}: {outcome:?}",
                command.id()
            );
        }
    }
    assert!(app.writes().is_empty());
    let audit = app.ports.state.audit_log();
    assert_eq!(audit.len(), 12);
    assert!(audit.iter().all(|r| r.outcome == AuditOutcome::Denied));
}
