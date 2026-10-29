//! Trigger, suspend and resume, end to end (E12-S08): the real init order with the app's own
//! adapters, its main window headless, a CronJob in a namespace of the test's own, and the user's
//! path: the Workloads section's CronJobs entry, the filter narrowing the table to the CronJob,
//! then `t` (trigger) and `s` (suspend, then resume) on its row. The commands go through the bus
//! and the guard (a confirmation first, audited in the SQLite state db), the Job really appears
//! in the cluster, owned by the CronJob, and completes; `spec.suspend` really flips.
//!
//! `cargo test -p oxikube --features integration --test kind_workloads` with
//! `OXIKUBE_TEST_CONTEXT` set (`cargo xtask kind-up`); without it the test returns at once. The
//! CronJob, its Jobs and the namespace are deleted at the end; no node is touched.
#![cfg(feature = "integration")]

mod kind_common;

use gpui::TestAppContext;
use kube::api::{ApiResource, DynamicObject, GroupVersionKind, ListParams, PostParams};
use oxikube::app_state::AppState;
use oxikube_app::store::FeedState;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_ports::AuditQuery;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_testkit::images::E2E_BUSYBOX;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_workspace::modal::DialogModal;
use oxikube_workspace::sidebar::SidebarPanel;
use serde_json::json;

use kind_common::{Launched, launch, on_cluster, wait, wait_value};

fn api(
    client: kube::Client,
    namespace: &str,
    kind: &str,
    plural: &str,
) -> kube::Api<DynamicObject> {
    let resource =
        ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("batch", "v1", kind), plural);
    kube::Api::namespaced_with(client, namespace, &resource)
}

fn create_cronjob(context: &str, namespace: &str, name: &str) {
    on_cluster(context, async |client| {
        let cronjob: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "batch/v1", "kind": "CronJob",
            "metadata": {"name": name},
            "spec": {
                // Only on 1 January: the controller never races the test's triggers.
                "schedule": "0 0 1 1 *",
                "jobTemplate": {"spec": {"backoffLimit": 0, "template": {"spec": {
                    "restartPolicy": "Never",
                    "containers": [{"name": "run", "image": E2E_BUSYBOX, "command": ["true"]}],
                }}}},
            },
        }))
        .expect("a CronJob");
        api(client, namespace, "CronJob", "cronjobs")
            .create(&PostParams::default(), &cronjob)
            .await
            .expect("create the CronJob");
    });
}

/// `(name, owner, succeeded)` of the Jobs in `namespace`.
fn jobs(context: &str, namespace: &str) -> Vec<(String, String, i64)> {
    on_cluster(context, async |client| {
        let list = api(client, namespace, "Job", "jobs")
            .list(&ListParams::default())
            .await
            .expect("list the Jobs");
        list.items
            .into_iter()
            .map(|job| {
                let name = job.metadata.name.clone().unwrap_or_default();
                let owner = job
                    .metadata
                    .owner_references
                    .as_ref()
                    .and_then(|owners| owners.first())
                    .map(|o| o.name.clone())
                    .unwrap_or_default();
                let succeeded = job.data["status"]["succeeded"].as_i64().unwrap_or(0);
                (name, owner, succeeded)
            })
            .collect()
    })
}

fn suspended(context: &str, namespace: &str, name: &str) -> Option<bool> {
    on_cluster(context, async |client| {
        let cronjob = api(client, namespace, "CronJob", "cronjobs")
            .get(name)
            .await
            .expect("the CronJob");
        cronjob.data["spec"]["suspend"].as_bool()
    })
}

#[gpui::test]
fn t_and_s_on_a_cronjobs_row_trigger_and_suspend_it_through_the_guard(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let ns = TestNamespace::create(&context).expect("namespace");
    let name = "e12-nightly";
    create_cronjob(&context, ns.name(), name);
    let Launched {
        mut vcx,
        tab,
        cluster,
        dir: _dir,
        ..
    } = launch(cx, &context);

    // The Workloads section lists CronJobs; its entry opens the table, which lists ours.
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let panel = vcx
        .update(|_, cx| inner.read(cx).panel::<SidebarPanel>())
        .expect("the sidebar");
    wait(&mut vcx, "the sidebar to list CronJobs", |vcx| {
        vcx.update(|_, cx| panel.read(cx).row("workloads/cronjobs").is_some())
    });
    vcx.update(|_, cx| panel.update(cx, |panel, cx| panel.activate("workloads/cronjobs", cx)));
    let table = wait_value(&mut vcx, "the CronJobs table to open", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .find(|table| &*table.read(cx).gvk().kind == "CronJob")
        })
    });
    // Narrow the table to the test's CronJob (other suites share the cluster) and put the cursor
    // on it.
    vcx.update(|window, cx| table.update(cx, |table, cx| table.set_filter_text(name, window, cx)));
    wait(&mut vcx, "the table to list the CronJob alone", |vcx| {
        vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                d.state() == &FeedState::Ready
                    && d.rows().iter().map(|r| r.name()).collect::<Vec<_>>() == [name]
            })
        })
    });
    vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("down");
    let entries = vcx.update(|_, cx| table.read(cx).action_entries(cx));
    for id in [
        CommandId::CRONJOB_TRIGGER,
        CommandId::CRONJOB_SUSPEND,
        CommandId::CRONJOB_RESUME,
    ] {
        assert!(
            entries.iter().any(|e| e.command() == id && e.is_enabled()),
            "{id} is offered: {:?}",
            entries.iter().map(|e| e.label.clone()).collect::<Vec<_>>()
        );
    }

    // `t`: the guard asks first, then the Job is created.
    let confirmation = |vcx: &mut gpui::VisualTestContext| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()
                .map(|dialog| dialog.read(cx).title().to_string())
        })
    };
    vcx.simulate_keystrokes("t");
    wait(&mut vcx, "the confirmation", |vcx| {
        confirmation(vcx).is_some()
    });
    assert_eq!(
        confirmation(&mut vcx).as_deref(),
        Some("Run the CronJob now?")
    );
    assert!(
        jobs(&context, ns.name()).is_empty(),
        "nothing before the answer"
    );
    vcx.simulate_keystrokes("enter");
    let made = wait_value(&mut vcx, "the Job to be created", |_| {
        jobs(&context, ns.name()).into_iter().next()
    });
    assert!(made.0.starts_with(&format!("{name}-manual-")), "{made:?}");
    assert_eq!(made.1, name, "owned by the CronJob");
    wait(&mut vcx, "the Job to complete", |_| {
        jobs(&context, ns.name())
            .iter()
            .all(|(_, _, succeeded)| *succeeded == 1)
    });

    // `s` suspends; the row follows the cluster, and `s` again resumes.
    vcx.simulate_keystrokes("s");
    wait(&mut vcx, "the confirmation", |vcx| {
        confirmation(vcx).is_some()
    });
    assert_eq!(confirmation(&mut vcx).as_deref(), Some("Suspend it?"));
    vcx.simulate_keystrokes("enter");
    wait(&mut vcx, "spec.suspend to be true", |_| {
        suspended(&context, ns.name(), name) == Some(true)
    });
    wait(&mut vcx, "the row to read as suspended", |vcx| {
        vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                d.rows()
                    .first()
                    .and_then(|r| r.resource())
                    .is_some_and(|r| {
                        r.get("/spec/suspend")
                            .is_some_and(|v| v.to_value() == json!(true))
                    })
            })
        })
    });
    vcx.simulate_keystrokes("s");
    wait(&mut vcx, "the confirmation", |vcx| {
        confirmation(vcx).is_some()
    });
    assert_eq!(confirmation(&mut vcx).as_deref(), Some("Resume it?"));
    vcx.simulate_keystrokes("enter");
    wait(&mut vcx, "spec.suspend to be false", |_| {
        suspended(&context, ns.name(), name) == Some(false)
    });

    // Read-only: the same keys are refused, nothing is created.
    let state = vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&cluster, true)
        .expect("the session is open");
    vcx.simulate_keystrokes("t");
    vcx.run_until_parked();
    assert!(confirmation(&mut vcx).is_none(), "refused before it asks");
    assert_eq!(jobs(&context, ns.name()).len(), 1, "still the one Job");

    // The guard audited each attempt, the trigger with the Job it made.
    let records = futures::executor::block_on(state.state().query_audit(&AuditQuery::default()))
        .expect("the audit log")
        .records;
    let ours: Vec<_> = records
        .iter()
        .filter(|r| &*r.target.name == name && r.cmd.starts_with("cronjob::"))
        .collect();
    let cmds: Vec<_> = ours.iter().map(|r| r.cmd.to_string()).collect();
    assert!(
        cmds.iter().any(|c| c == "cronjob::Trigger")
            && cmds.iter().any(|c| c == "cronjob::Suspend")
            && cmds.iter().any(|c| c == "cronjob::Resume"),
        "{records:?}"
    );
    let trigger = ours.iter().find(|r| &*r.cmd == "cronjob::Trigger").unwrap();
    assert_eq!(trigger.outcome, AuditOutcome::Succeeded);
    assert_eq!(trigger.initiator, Initiator::Ui);
    assert_eq!(
        trigger.detail.as_deref(),
        Some(format!("job={}", made.0).as_str())
    );
}
