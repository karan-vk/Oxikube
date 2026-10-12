//! The CronJob and Job keys (E12-S08), pressed in real tables with the shipped keymap: `t`
//! triggers a CronJob, `s` suspends a CronJob or a Job, or resumes it when it is suspended. Each
//! dispatches the command the palette, the context menu and an agent run; the guard's read-only
//! block applies to all of them.

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use serde_json::json;

use super::fixture::{Fixture, cluster};
use crate::actions::tests::toasts;
use crate::table::ResourceTable;

fn kind(kind: &str, plural: &str) -> ResourceKind {
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

fn cronjob_kind() -> ResourceKind {
    kind("CronJob", "cronjobs")
}

fn job_kind() -> ResourceKind {
    kind("Job", "jobs")
}

fn object(api_kind: &str, name: &str, suspend: bool) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "batch/v1", "kind": api_kind,
        "metadata": {"name": name, "namespace": "x", "uid": format!("{name}-uid"),
                     "resourceVersion": "1", "creationTimestamp": "2026-10-01T00:00:00Z"},
        "spec": {"suspend": suspend, "schedule": "0 3 * * *"},
    }))
    .unwrap()
}

fn target(kind: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("batch", "v1", kind), "x", name)
}

/// A connected cluster serving CronJobs and Jobs, and the table of `open` with its first row on
/// the cursor.
fn open(
    cx: &mut TestAppContext,
    objects: impl IntoIterator<Item = Resource>,
    open: ResourceKind,
) -> (Fixture, Entity<ResourceTable>) {
    let mut f = Fixture::with_exec(cx);
    let ports = f.ports();
    ports.discovery.set_kinds([cronjob_kind(), job_kind()]);
    for object in objects {
        ports.resources.insert(object);
    }
    futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    let table = f.open(open);
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    f.dispatcher.clear();
    (f, table)
}

fn messages(f: &mut Fixture) -> Vec<String> {
    toasts(f)
        .into_iter()
        .map(|t| t.message.to_string())
        .collect()
}

/// Sends the dispatched commands through the real bus: the keys only dispatch, but the guard's
/// answer (a confirmation) is what the user sees, so the tests that need it use the entries.
fn sent(f: &Fixture) -> Vec<Command> {
    f.dispatcher.sent()
}

#[gpui::test]
fn t_triggers_the_cursor_rows_cronjob(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, [object("CronJob", "nightly", false)], cronjob_kind());
    f.keys(&table, "t");
    assert_eq!(
        sent(&f),
        [Command::CronJobTrigger {
            target: target("CronJob", "nightly")
        }]
    );
    assert_eq!(messages(&mut f), [] as [&str; 0]);
}

#[gpui::test]
fn s_suspends_a_running_cronjob_and_resumes_a_suspended_one(cx: &mut TestAppContext) {
    let (mut f, table) = open(
        cx,
        [
            object("CronJob", "a-running", false),
            object("CronJob", "b-paused", true),
        ],
        cronjob_kind(),
    );
    assert_eq!(f.selected(&table), ["a-running"]);
    f.keys(&table, "s");
    f.keys(&table, "down");
    assert_eq!(f.selected(&table), ["b-paused"]);
    f.keys(&table, "s");
    assert_eq!(
        sent(&f),
        [
            Command::CronJobSuspend {
                target: target("CronJob", "a-running")
            },
            Command::CronJobResume {
                target: target("CronJob", "b-paused")
            },
        ]
    );
}

#[gpui::test]
fn s_suspends_and_resumes_jobs_with_the_job_commands(cx: &mut TestAppContext) {
    let (mut f, table) = open(
        cx,
        [object("Job", "a-job", false), object("Job", "b-job", true)],
        job_kind(),
    );
    f.keys(&table, "s");
    f.keys(&table, "down");
    f.keys(&table, "s");
    assert_eq!(
        sent(&f),
        [
            Command::JobSuspend {
                target: target("Job", "a-job")
            },
            Command::JobResume {
                target: target("Job", "b-job")
            },
        ]
    );
}

#[gpui::test]
fn t_on_a_job_table_does_nothing(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, [object("Job", "a-job", false)], job_kind());
    f.keys(&table, "t");
    assert!(
        sent(&f).is_empty(),
        "a Job has nothing to trigger: {:?}",
        sent(&f)
    );
}

#[gpui::test]
fn the_keys_are_text_while_the_filter_field_has_the_focus(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, [object("CronJob", "nightly", false)], cronjob_kind());
    f.keys(&table, "/ t s");
    let verbs: Vec<_> = sent(&f)
        .into_iter()
        .filter(|c| !matches!(c, Command::TableFocusFilter { .. }))
        .collect();
    assert_eq!(verbs, [], "typed into the filter, not run");
}

#[gpui::test]
fn the_menu_lists_trigger_suspend_and_resume_for_cronjobs_only(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, [object("CronJob", "nightly", false)], cronjob_kind());
    let labels = |f: &mut Fixture, table: &Entity<ResourceTable>| -> Vec<String> {
        f.vcx
            .update(|_, cx| table.read(cx).action_entries(cx))
            .into_iter()
            .map(|entry| entry.label)
            .collect()
    };
    assert_eq!(
        labels(&mut f, &table),
        ["Trigger", "Suspend", "Resume", "Edit labels", "Delete"]
    );
    let jobs = f.open(job_kind());
    assert_eq!(
        labels(&mut f, &jobs),
        ["Suspend", "Resume", "Edit labels", "Delete"]
    );
}

#[gpui::test]
fn a_read_only_cluster_greys_out_every_action_with_its_reason(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx, [object("CronJob", "nightly", false)], cronjob_kind());
    f.sessions.set_read_only(&cluster(), true).unwrap();
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    for id in [
        CommandId::CRONJOB_TRIGGER,
        CommandId::CRONJOB_SUSPEND,
        CommandId::CRONJOB_RESUME,
    ] {
        let entry = entries
            .iter()
            .find(|e| e.command() == id)
            .unwrap_or_else(|| panic!("{id} is listed"));
        assert!(!entry.is_enabled(), "{id}");
        assert_eq!(entry.reason().as_deref(), Some("This cluster is read-only"));
    }
    // The key says why instead of dispatching.
    f.keys(&table, "t");
    assert!(sent(&f).is_empty());
    assert!(
        messages(&mut f).iter().any(|m| m.contains("read-only")),
        "{:?}",
        messages(&mut f)
    );
}

#[gpui::test]
fn with_several_rows_selected_a_key_acts_on_the_cursor_row_and_says_so(cx: &mut TestAppContext) {
    let (mut f, table) = open(
        cx,
        [
            object("CronJob", "a-one", false),
            object("CronJob", "b-two", false),
        ],
        cronjob_kind(),
    );
    f.keys(&table, "shift-down");
    assert_eq!(f.selected(&table), ["a-one", "b-two"]);
    f.keys(&table, "t");
    assert_eq!(sent(&f).len(), 1, "{:?}", sent(&f));
    assert!(
        messages(&mut f)
            .iter()
            .any(|m| m.contains("acts on the cursor row")),
        "{:?}",
        messages(&mut f)
    );
}

#[gpui::test]
fn the_palette_lists_the_cronjob_commands_for_a_cronjob_table(cx: &mut TestAppContext) {
    use oxikube_workspace::command_surface::CommandSurface as _;
    let (mut f, table) = open(cx, [object("CronJob", "nightly", false)], cronjob_kind());
    let own = f.vcx.update(|_, cx| table.read(cx).own_commands(cx));
    for id in [
        CommandId::CRONJOB_TRIGGER,
        CommandId::CRONJOB_SUSPEND,
        CommandId::CRONJOB_RESUME,
    ] {
        assert!(own.contains(&id), "{id} in {own:?}");
    }
    assert!(!own.contains(&CommandId::JOB_SUSPEND));
}
