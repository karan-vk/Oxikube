//! Trigger, suspend and resume (E12-S08) against the real adapter: `cronjob::Trigger`,
//! `cronjob::Suspend` / `Resume` and `job::Suspend` / `Resume` through the `CommandBus` and
//! `MutationGuard` on a kind cluster, no UI.
//!
//! * a trigger creates a Job the API server names `<cronjob>-manual-…`, owned by the CronJob and
//!   marked `cronjob.kubernetes.io/instantiate: manual`, and the Job completes;
//! * suspend and resume flip `spec.suspend` of a CronJob and of a Job (and the Job controller
//!   reports the `Suspended` condition);
//! * a finished Job is not resumed, and the refusal sends nothing;
//! * a dispatch that is itself a dry run creates and changes nothing, and a read-only session is
//!   refused before any request;
//! * every attempt leaves one audit record, the trigger's naming the Job it made.
//!
//! Every object lives in the test's own `oxi-test-<rand>` namespace; nothing touches a node.

use std::sync::Arc;

use k8s_openapi::api::batch::v1::{CronJob, Job};
use kube::Api;
use kube::api::{ListParams, PostParams};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::workloads::register_commands;
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort, images};
use serde_json::json;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

fn cronjob_ref(cluster: &ClusterId, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("batch", "v1", "CronJob"),
        namespace,
        name,
    )
}

fn job_ref(cluster: &ClusterId, namespace: &str, name: &str) -> ResourceRef {
    ResourceRef::namespaced(
        cluster.clone(),
        Gvk::new("batch", "v1", "Job"),
        namespace,
        name,
    )
}

/// The CronJob under test: it would only run on 1 January, so the controller never races the
/// triggers. Its Jobs run `true` and finish at once.
fn cronjob(name: &str) -> CronJob {
    serde_json::from_value(json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": name},
        "spec": {
            "schedule": "0 0 1 1 *",
            "jobTemplate": {
                "metadata": {"labels": {"oxikube.test/role": "template"}},
                "spec": {"backoffLimit": 0, "template": {"spec": {
                    "restartPolicy": "Never",
                    "containers": [{
                        "name": "run", "image": images::E2E_BUSYBOX, "command": ["true"],
                    }],
                }}},
            },
        },
    }))
    .expect("a valid CronJob")
}

/// A Job that keeps running (`sleep`), created suspended or not.
fn sleeper(name: &str, suspend: bool) -> Job {
    serde_json::from_value(json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": name},
        "spec": {"suspend": suspend, "backoffLimit": 0, "template": {"spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [{
                "name": "run", "image": images::E2E_BUSYBOX, "command": ["sleep", "3600"],
            }],
        }}},
    }))
    .expect("a valid Job")
}

/// Dispatches `command` as the UI and answers the simple confirmation the guard asks for.
async fn run(
    bus: &CommandBus,
    command: Command,
    dry_run: bool,
) -> Result<oxikube_app::CommandOutput, DispatchError> {
    let ctx = DispatchContext::new(Initiator::Ui, "tester").with_dry_run(dry_run);
    let request = match bus.dispatch(command.clone(), ctx.clone()).await? {
        Outcome::NeedsConfirmation(request) => request,
        Outcome::Completed(_) => panic!("a low-risk mutation still asks"),
    };
    bus.dispatch(
        command,
        ctx.with_confirmation(Confirmation::simple(request.token)),
    )
    .await
    .map(|outcome| outcome.completed().expect("the handler ran"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trigger_suspend_and_resume_on_a_kind_cluster() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin_client = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let cronjobs = Api::<CronJob>::namespaced(admin_client.clone(), ns.name());
    let jobs = Api::<Job>::namespaced(admin_client.clone(), ns.name());
    cronjobs
        .create(&PostParams::default(), &cronjob("nightly"))
        .await
        .expect("create the cronjob");
    let uid = cronjobs
        .get("nightly")
        .await
        .unwrap()
        .metadata
        .uid
        .expect("uid");

    let admin = catalog_entry(&kind.context);
    let manager = ClusterSessionManager::with_config(
        Arc::new(KubeConnector::new(
            kind.kubeconfig.clone(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        )),
        Arc::new(FakeClusterSourcePort::new()),
        Arc::new(TokioClock),
        SessionManagerConfig::default(),
    );
    manager.open(&admin, SessionOptions::default());
    manager.connect(&admin.cluster).await.expect("connect");
    let state = Arc::new(FakeStatePort::new());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::workloads", |r| {
            register_commands(r, manager.clone())
        })
        .expect("register the workload commands");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let cluster = admin.cluster.clone();

    // --- a dry run creates nothing ---------------------------------------------------------------
    let trigger = Command::CronJobTrigger {
        target: cronjob_ref(&cluster, ns.name(), "nightly"),
    };
    run(&bus, trigger.clone(), true).await.expect("dry run");
    assert!(
        jobs.list(&ListParams::default())
            .await
            .unwrap()
            .items
            .is_empty(),
        "a dry run creates no Job"
    );

    // --- a trigger creates the Job and it completes -----------------------------------------------
    let output = run(&bus, trigger.clone(), false).await.expect("trigger");
    let data = output.data.expect("data");
    let job_name = data["job"]["name"]
        .as_str()
        .expect("the Job's name")
        .to_owned();
    assert!(job_name.starts_with("nightly-manual-"), "{job_name}");
    assert_eq!(data["open"], data["job"]);
    let job = jobs.get(&job_name).await.expect("the Job exists");
    let owner = &job.metadata.owner_references.clone().expect("owner")[0];
    assert_eq!(
        (owner.kind.as_str(), owner.name.as_str()),
        ("CronJob", "nightly")
    );
    assert_eq!(owner.uid, uid);
    assert_eq!(owner.controller, Some(true));
    assert_eq!(
        job.metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get("cronjob.kubernetes.io/instantiate"))
            .map(String::as_str),
        Some("manual")
    );
    assert_eq!(
        job.metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("oxikube.test/role"))
            .map(String::as_str),
        Some("template"),
        "the template's labels carry over"
    );
    crate::eventually(
        "the triggered Job to complete",
        || format!("{job:?}"),
        || async {
            jobs.get(&job_name)
                .await
                .ok()
                .and_then(|job| job.status)
                .is_some_and(|status| status.succeeded == Some(1))
        },
    )
    .await;

    // --- a finished Job cannot be resumed; nothing is sent ----------------------------------------
    let resume_done = Command::JobResume {
        target: job_ref(&cluster, ns.name(), &job_name),
    };
    let error = run(&bus, resume_done, false).await.unwrap_err();
    assert!(error.to_string().contains("cannot be resumed"), "{error}");

    // --- suspend and resume a CronJob ---------------------------------------------------------------
    let target = cronjob_ref(&cluster, ns.name(), "nightly");
    run(
        &bus,
        Command::CronJobSuspend {
            target: target.clone(),
        },
        false,
    )
    .await
    .expect("suspend");
    assert_eq!(
        cronjobs.get("nightly").await.unwrap().spec.suspend,
        Some(true)
    );
    run(&bus, Command::CronJobResume { target }, false)
        .await
        .expect("resume");
    assert_eq!(
        cronjobs.get("nightly").await.unwrap().spec.suspend,
        Some(false)
    );

    // --- suspend and resume a running Job -------------------------------------------------------------
    jobs.create(&PostParams::default(), &sleeper("sleeper", false))
        .await
        .expect("create the Job");
    let target = job_ref(&cluster, ns.name(), "sleeper");
    run(
        &bus,
        Command::JobSuspend {
            target: target.clone(),
        },
        false,
    )
    .await
    .expect("suspend the job");
    assert_eq!(
        jobs.get("sleeper").await.unwrap().spec.unwrap().suspend,
        Some(true)
    );
    crate::eventually(
        "the Job controller to report Suspended",
        String::new,
        || async {
            jobs.get("sleeper")
                .await
                .ok()
                .and_then(|job| job.status)
                .and_then(|status| status.conditions)
                .is_some_and(|conditions| {
                    conditions
                        .iter()
                        .any(|c| c.type_ == "Suspended" && c.status == "True")
                })
        },
    )
    .await;
    run(&bus, Command::JobResume { target }, false)
        .await
        .expect("resume the job");
    assert_eq!(
        jobs.get("sleeper").await.unwrap().spec.unwrap().suspend,
        Some(false)
    );

    // --- read-only: refused before any request ------------------------------------------------------
    let before = jobs.list(&ListParams::default()).await.unwrap().items.len();
    manager.set_read_only(&cluster, true).unwrap();
    let error = bus
        .dispatch(trigger, DispatchContext::new(Initiator::Agent, "agent"))
        .await
        .unwrap_err();
    assert!(matches!(error, DispatchError::ReadOnly { .. }), "{error:?}");
    manager.set_read_only(&cluster, false).unwrap();
    assert_eq!(
        jobs.list(&ListParams::default()).await.unwrap().items.len(),
        before
    );

    // --- the audit trail ------------------------------------------------------------------------------
    let audit = state.audit_log();
    let triggers: Vec<_> = audit
        .iter()
        .filter(|r| &*r.cmd == "cronjob::Trigger")
        .collect();
    assert_eq!(
        triggers.len(),
        4,
        "a dry run, the guard's dry run before the trigger (E19-S07), the trigger and the \
         refused one: {audit:?}"
    );
    assert!(triggers[0].dry_run && triggers[1].dry_run);
    assert_eq!(
        triggers[2].detail.as_deref(),
        Some(format!("job={job_name}").as_str())
    );
    assert_eq!(triggers[2].outcome, AuditOutcome::Succeeded);
    assert_eq!(triggers[3].outcome, AuditOutcome::Denied);
    assert_eq!(triggers[3].initiator, Initiator::Agent);
}
