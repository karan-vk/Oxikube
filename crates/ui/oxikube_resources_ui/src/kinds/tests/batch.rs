//! Jobs and CronJobs (E12-S08) in the kind registry: their specs, their sidebar slots and the
//! columns their tables open with.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::{ResourceKind, VerbSet};

use crate::kinds::{KindSpecs, cronjob, job};
use crate::table::tests::fixture::{Fixture, cluster};

fn served(kind: &str, plural: &str) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new("batch", "v1", kind),
        preferred: true,
        plural: plural.into(),
        singular: kind.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete", "patch", "create"]),
        namespaced: true,
    }
}

#[gpui::test]
fn the_specs_are_registered_with_their_actions_in_the_workloads_section(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    f.vcx.update(|_, cx| {
        let cronjob = KindSpecs::kind_for(cx, &Gvk::new("batch", "v1", "CronJob"));
        assert_eq!(
            cronjob.actions,
            [
                CommandId::CRONJOB_TRIGGER,
                CommandId::CRONJOB_SUSPEND,
                CommandId::CRONJOB_RESUME
            ]
        );
        let slot = cronjob.sidebar.expect("a sidebar slot");
        assert_eq!(slot.section.id, "workloads");
        let job = KindSpecs::kind_for(cx, &Gvk::new("batch", "v1", "Job"));
        assert_eq!(job.actions, [CommandId::JOB_SUSPEND, CommandId::JOB_RESUME]);
        assert!(
            job.sidebar.expect("a slot").rank < slot.rank,
            "Jobs come before CronJobs"
        );
        // The registered actions are the ones the module offers as row actions.
        let offered: Vec<_> = cronjob::row_actions()
            .into_iter()
            .chain(job::row_actions())
            .map(|spec| spec.command)
            .collect();
        for id in cronjob::ACTIONS.iter().chain(job::ACTIONS) {
            assert!(offered.contains(id), "{id}");
        }
    });
}

#[gpui::test]
fn the_cronjob_table_opens_with_schedule_timezone_suspend_active_and_last_schedule(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    f.ports()
        .discovery
        .set_kinds([served("CronJob", "cronjobs"), served("Job", "jobs")]);
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    let table = f.open(served("CronJob", "cronjobs"));
    let columns = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    let ids: Vec<&str> = columns.iter().map(ColumnId::as_str).collect();
    assert_eq!(
        ids,
        [
            "name",
            "namespace",
            "schedule",
            "timezone",
            "suspend",
            "active",
            "last-schedule",
            "age"
        ]
    );

    let jobs = f.open(served("Job", "jobs"));
    let columns = f
        .vcx
        .update(|_, cx| jobs.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    let ids: Vec<&str> = columns.iter().map(ColumnId::as_str).collect();
    assert_eq!(
        ids,
        [
            "name",
            "namespace",
            "status",
            "completions",
            "duration",
            "conditions",
            "age"
        ]
    );
}
