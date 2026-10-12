//! Pod actions, end to end (E12-S03): the real init order with the app's own adapters, its main
//! window headless, three pods in a namespace of the test's own (one Completed, one Failed, one
//! running), and the user's path: the pods table narrowed to them, `enter` for the pod detail with
//! its panel, `z` to sweep the finished pods and `ctrl-k` to force delete the running one. The
//! commands go through the bus and the guard (a confirmation first, the pod's name typed for the
//! force delete, audited in the SQLite state db), the pods really leave the cluster, and a
//! read-only session is refused before anything is asked.
//!
//! `cargo test -p oxikube --features integration --test kind_pods` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once. Every pod and the namespace
//! are the test's own; no node is touched and nothing is evicted outside the namespace.
#![cfg(feature = "integration")]

mod kind_common;

use std::time::Duration;

use gpui::TestAppContext;
use kube::api::{ApiResource, DynamicObject, GroupVersionKind, PostParams};
use oxikube::app_state::AppState;
use oxikube_app::store::FeedState;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::CommandId;
use oxikube_ports::AuditQuery;
use oxikube_resources_ui::detail::DetailDrawer;
use oxikube_resources_ui::kinds::pods::PodPanel;
use oxikube_testkit::images::E2E_BUSYBOX;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_workspace::modal::DialogModal;
use serde_json::json;

use kind_common::{Launched, launch, on_cluster, wait, wait_value};

fn pods_api(client: kube::Client, namespace: &str) -> kube::Api<DynamicObject> {
    let resource =
        ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("", "v1", "Pod"), "pods");
    kube::Api::namespaced_with(client, namespace, &resource)
}

/// A pod that runs `command` and stops (grace period 0, so a delete is quick).
fn create_pod(context: &str, namespace: &str, name: &str, command: &[&str]) {
    on_cluster(context, async |client| {
        let pod: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": name},
            "spec": {
                "restartPolicy": "Never",
                "terminationGracePeriodSeconds": 0,
                "containers": [{"name": "run", "image": E2E_BUSYBOX, "command": command}],
            },
        }))
        .expect("a pod");
        pods_api(client, namespace)
            .create(&PostParams::default(), &pod)
            .await
            .expect("create the pod");
    });
}

/// `Some(phase)` while the pod exists, `None` once it is gone.
fn phase(context: &str, namespace: &str, name: &str) -> Option<String> {
    on_cluster(context, async |client| {
        pods_api(client, namespace)
            .get_opt(name)
            .await
            .expect("read the pod")
            .map(|pod| {
                pod.data["status"]["phase"]
                    .as_str()
                    .unwrap_or("Pending")
                    .to_owned()
            })
    })
}

#[gpui::test]
fn z_and_ctrl_k_on_the_pods_table_sweep_and_force_delete_through_the_guard(
    cx: &mut TestAppContext,
) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let ns = TestNamespace::create(&context).expect("namespace");
    create_pod(&context, ns.name(), "e12p-done", &["true"]);
    create_pod(&context, ns.name(), "e12p-broken", &["false"]);
    create_pod(&context, ns.name(), "e12p-sleeper", &["sleep", "3600"]);
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        dir: _dir,
    } = launch(cx, &context);
    wait(&mut vcx, "the pods to reach their phases", |_| {
        phase(&context, ns.name(), "e12p-done").as_deref() == Some("Succeeded")
            && phase(&context, ns.name(), "e12p-broken").as_deref() == Some("Failed")
            && phase(&context, ns.name(), "e12p-sleeper").as_deref() == Some("Running")
    });

    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    // Narrow the table to the test's pods (other suites share the cluster).
    vcx.update(|window, cx| {
        table.update(cx, |table, cx| table.set_filter_text("e12p-", window, cx))
    });
    wait(&mut vcx, "the table to list the three pods", |vcx| {
        vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                d.state() == &FeedState::Ready
                    && d.rows().iter().map(|r| r.name()).collect::<Vec<_>>()
                        == ["e12p-broken", "e12p-done", "e12p-sleeper"]
            })
        })
    });
    vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("down down down");
    assert_eq!(
        vcx.update(|_, cx| table.read(cx).selected_refs(cx))
            .iter()
            .map(|r| r.name.to_string())
            .collect::<Vec<_>>(),
        ["e12p-sleeper"]
    );

    // The pod's own actions are offered.
    let entries = vcx.update(|_, cx| table.read(cx).action_entries(cx));
    for id in [
        CommandId::POD_EVICT,
        CommandId::POD_DELETE,
        CommandId::POD_FORCE_DELETE,
        CommandId::POD_SANITIZE,
        CommandId::POD_JUMP_TO_OWNER,
        CommandId::POD_SHOW_NODE,
    ] {
        assert!(
            entries.iter().any(|e| e.command() == id && e.is_enabled()),
            "{id} is offered: {:?}",
            entries.iter().map(|e| e.label.clone()).collect::<Vec<_>>()
        );
    }

    // Read-only: the same keys are refused before anything is asked (the menu greys the
    // actions out with the reason, and the guard refuses them for every other initiator, as the
    // app's tests show).
    let state = vcx.update(|_, cx| AppState::global(cx));
    state
        .services()
        .sessions
        .set_read_only(&cluster, true)
        .expect("the session is open");
    let greyed = vcx.update(|_, cx| table.read(cx).action_entries(cx));
    assert!(
        greyed
            .iter()
            .filter(|e| e.command() == CommandId::POD_FORCE_DELETE)
            .all(|e| !e.is_enabled()),
        "force delete is greyed out on a read-only cluster"
    );
    vcx.simulate_keystrokes("z");
    vcx.simulate_keystrokes("ctrl-k");
    vcx.run_until_parked();
    std::thread::sleep(Duration::from_millis(300));
    vcx.run_until_parked();
    assert!(
        !vcx.update(|_, cx| inner.read(cx).modal_layer().read(cx).has_active_modal()),
        "refused before it asks"
    );
    for name in ["e12p-done", "e12p-broken", "e12p-sleeper"] {
        assert!(
            phase(&context, ns.name(), name).is_some(),
            "{name} is untouched"
        );
    }
    state
        .services()
        .sessions
        .set_read_only(&cluster, false)
        .expect("the session is open");

    // `enter` opens the detail; its panel shows the node and the container.
    vcx.simulate_keystrokes("enter");
    let drawer = wait_value(&mut vcx, "the detail drawer", |vcx| {
        vcx.update(|_, cx| inner.read(cx).panel::<DetailDrawer>())
    });
    let panel = wait_value(&mut vcx, "the pod's panel", |vcx| {
        vcx.update(|_, cx| {
            let view = drawer.read(cx).view()?.clone();
            let panel = view.read(cx).kind_panel().cloned()?;
            panel.downcast::<PodPanel>().ok()
        })
    });
    wait(&mut vcx, "the node row", |vcx| {
        vcx.update(|_, cx| {
            panel
                .read(cx)
                .sections()
                .iter()
                .any(|s| s.id.key() == "node" && s.count == 1)
        })
    });
    // Back to the table.
    vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.run_until_parked();

    let dialog = |vcx: &mut gpui::VisualTestContext| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DialogModal>()
        })
    };

    // `z`: the guard asks (with the count), then the finished pods go and the running one stays.
    vcx.simulate_keystrokes("z");
    wait(&mut vcx, "the confirmation", |vcx| dialog(vcx).is_some());
    let asking = dialog(&mut vcx).expect("a dialog");
    let (title, message) = vcx.update(|_, cx| {
        let d = asking.read(cx);
        (
            d.title().to_string(),
            d.message_text()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
    });
    assert_eq!(title, "Delete finished pods?");
    assert!(
        message.contains("Delete 2 Completed and Failed pods"),
        "{message}"
    );
    assert_eq!(
        phase(&context, ns.name(), "e12p-done").as_deref(),
        Some("Succeeded"),
        "nothing before the answer"
    );
    vcx.simulate_keystrokes("enter");
    wait(&mut vcx, "the finished pods to be deleted", |_| {
        phase(&context, ns.name(), "e12p-done").is_none()
            && phase(&context, ns.name(), "e12p-broken").is_none()
    });
    assert_eq!(
        phase(&context, ns.name(), "e12p-sleeper").as_deref(),
        Some("Running"),
        "a running pod is not swept"
    );

    // `ctrl-k`: the pod's name is typed before anything is sent.
    wait(&mut vcx, "the table to list the sleeper alone", |vcx| {
        vcx.update(|_, cx| {
            table.read(cx).read_rows(cx, |d| {
                d.rows().iter().map(|r| r.name()).collect::<Vec<_>>() == ["e12p-sleeper"]
            })
        })
    });
    vcx.simulate_keystrokes("down");
    vcx.simulate_keystrokes("ctrl-k");
    wait(&mut vcx, "the force delete confirmation", |vcx| {
        dialog(vcx).is_some()
    });
    let asking = dialog(&mut vcx).expect("a dialog");
    assert_eq!(
        vcx.update(|_, cx| asking.read(cx).title().to_string()),
        "Force delete the pod?"
    );
    assert_eq!(
        vcx.update(|_, cx| asking.read(cx).expected_text().map(ToString::to_string))
            .as_deref(),
        Some("e12p-sleeper")
    );
    vcx.update(|window, cx| asking.update(cx, |d, cx| d.press_confirm(window, cx)));
    vcx.run_until_parked();
    assert_eq!(
        phase(&context, ns.name(), "e12p-sleeper").as_deref(),
        Some("Running"),
        "an empty answer deletes nothing"
    );
    vcx.update(|window, cx| {
        asking.update(cx, |d, cx| d.type_text("e12p-sleeper", window, cx));
        asking.update(cx, |d, cx| d.press_confirm(window, cx));
    });
    wait(&mut vcx, "the pod to leave the API at once", |_| {
        phase(&context, ns.name(), "e12p-sleeper").is_none()
    });

    // The guard audited every attempt: one record per swept pod under the batch, and the force
    // delete with its grace period.
    let records = futures::executor::block_on(state.state().query_audit(&AuditQuery::default()))
        .expect("the audit log");
    let ours: Vec<_> = records
        .iter()
        .filter(|r| r.target.name.starts_with("e12p-") || &*r.cmd == "pod::Sanitize")
        .collect();
    let swept: Vec<_> = ours
        .iter()
        .filter(|r| &*r.cmd == "pod::Delete" && r.outcome == AuditOutcome::Succeeded)
        .collect();
    assert_eq!(swept.len(), 2, "{records:?}");
    let batch = swept[0].detail.clone().expect("a batch detail");
    assert!(batch.starts_with("batch=sweep-"), "{batch}");
    assert!(swept.iter().all(|r| r.detail.as_deref() == Some(&*batch)));
    assert!(swept.iter().all(|r| r.initiator == Initiator::Ui));
    let force = ours
        .iter()
        .find(|r| &*r.cmd == "pod::ForceDelete")
        .expect("the force delete is audited");
    assert_eq!(force.outcome, AuditOutcome::Succeeded);
    assert_eq!(force.detail.as_deref(), Some("grace=0"));
    // The refused keys never reached the guard (the actions were greyed out): one sweep record.
    assert_eq!(
        records
            .iter()
            .filter(|r| &*r.cmd == "pod::Sanitize")
            .count(),
        1,
        "{records:?}"
    );
}
