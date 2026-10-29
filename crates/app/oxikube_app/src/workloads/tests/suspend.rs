//! Suspend and resume: the merge patches, and the Job that cannot be resumed.

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use serde_json::json;

use super::*;

fn suspend(target: ResourceRef) -> Command {
    if &*target.gvk.kind == "Job" {
        Command::JobSuspend { target }
    } else {
        Command::CronJobSuspend { target }
    }
}

fn resume(target: ResourceRef) -> Command {
    if &*target.gvk.kind == "Job" {
        Command::JobResume { target }
    } else {
        Command::CronJobResume { target }
    }
}

#[test]
fn suspending_a_cronjob_merge_patches_spec_suspend_after_a_dry_run() {
    let t = Workloads::new(false);
    let output = t
        .run(suspend(cronjob_ref("nightly")), Initiator::Ui)
        .expect("suspend");
    // The guard's dry run before it asks (E19-S07), the handler's, the patch.
    assert_eq!(
        t.writes(),
        [
            ("CronJob".to_owned(), "patch", true),
            ("CronJob".to_owned(), "patch", true),
            ("CronJob".to_owned(), "patch", false)
        ]
    );
    assert_eq!(t.body(2), json!({"spec": {"suspend": true}}));
    assert_eq!(
        output.message.as_deref(),
        Some("Suspended CronJob default/nightly")
    );
    assert_eq!(output.data.unwrap()["suspended"], true);

    let live = t.objects_of(&Gvk::new("batch", "v1", "CronJob"));
    assert_eq!(
        live[0].get("/spec/suspend").unwrap().to_value(),
        json!(true)
    );
    // The rest of the CronJob is untouched by a merge patch.
    assert_eq!(
        live[0].get("/spec/schedule").unwrap().to_value(),
        json!("0 3 * * *")
    );
}

#[test]
fn resuming_a_cronjob_patches_it_back() {
    let t = Workloads::new(false);
    t.run(suspend(cronjob_ref("nightly")), Initiator::Ui)
        .unwrap();
    let output = t
        .run(resume(cronjob_ref("nightly")), Initiator::Ui)
        .expect("resume");
    assert_eq!(t.body(5), json!({"spec": {"suspend": false}}));
    assert_eq!(
        output.message.as_deref(),
        Some("Resumed CronJob default/nightly")
    );
    let live = t.objects_of(&Gvk::new("batch", "v1", "CronJob"));
    assert_eq!(
        live[0].get("/spec/suspend").unwrap().to_value(),
        json!(false)
    );
}

#[test]
fn a_running_job_can_be_suspended_and_a_suspended_one_resumed() {
    let t = Workloads::new(false);
    t.port().insert(job(
        "import",
        json!({"suspend": false}),
        json!({"active": 1}),
    ));
    t.run(suspend(job_ref("import")), Initiator::Ui)
        .expect("suspend");
    assert_eq!(t.body(2), json!({"spec": {"suspend": true}}));
    t.run(resume(job_ref("import")), Initiator::Ui)
        .expect("resume");
    assert_eq!(t.body(5), json!({"spec": {"suspend": false}}));
    let kinds: Vec<_> = t.writes().into_iter().map(|w| w.0).collect();
    assert_eq!(kinds, ["Job"; 6]);
}

#[test]
fn a_finished_job_cannot_be_resumed() {
    let t = Workloads::new(false);
    for (name, condition) in [("done", "Complete"), ("broken", "Failed")] {
        t.port().insert(job(
            name,
            json!({"suspend": true}),
            json!({"conditions": [{"type": condition, "status": "True"}]}),
        ));
        let error = t.run(resume(job_ref(name)), Initiator::Ui).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("has finished and cannot be resumed"),
            "{error}"
        );
    }
    assert!(t.writes().is_empty(), "nothing was sent: {:?}", t.writes());
    // The refusal is audited as a failed attempt, not lost.
    let outcomes: Vec<_> = t.h.audit().iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [AuditOutcome::Failed, AuditOutcome::Failed]);
}

#[test]
fn suspending_a_job_that_is_already_finished_is_left_to_the_server() {
    // Only resume is refused client-side: kubectl patch of a finished job is the server's call.
    let t = Workloads::new(false);
    t.port().insert(job(
        "done",
        json!({}),
        json!({"conditions": [{"type": "Complete", "status": "True"}]}),
    ));
    t.run(suspend(job_ref("done")), Initiator::Ui)
        .expect("sent");
    assert_eq!(
        t.writes().len(),
        3,
        "the guard's dry run, the handler's, the patch"
    );
}

#[test]
fn only_cronjobs_and_jobs_suspend() {
    let t = Workloads::new(false);
    let deployment = ResourceRef::namespaced(
        id("a"),
        Gvk::new("apps", "v1", "Deployment"),
        "default",
        "web",
    );
    let error = t
        .run(Command::JobSuspend { target: deployment }, Initiator::Ui)
        .unwrap_err();
    assert!(
        error.to_string().contains("only CronJobs and Jobs can"),
        "{error}"
    );
    assert!(t.writes().is_empty());
}

#[test]
fn a_suspend_dry_run_changes_nothing() {
    let t = Workloads::new(false);
    let output = t.dry_run(suspend(cronjob_ref("nightly"))).expect("dry run");
    assert_eq!(t.writes(), [("CronJob".to_owned(), "patch", true)]);
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: CronJob default/nightly can be suspended")
    );
    let live = t.objects_of(&Gvk::new("batch", "v1", "CronJob"));
    assert_eq!(
        live[0].get("/spec/suspend").unwrap().to_value(),
        json!(false)
    );
}

use oxikube_domain::audit::AuditOutcome;
