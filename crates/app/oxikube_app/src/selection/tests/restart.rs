//! `selection::Restart`: every rollable workload is stamped; anything else refuses the batch
//! before a request is sent.

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditOutcome, Initiator};

use super::*;
use crate::selection::BulkReport;

const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

fn statefulset(name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": name, "namespace": "shop", "uid": format!("{name}-uid")},
        "spec": {"template": {"spec": {"containers": [{"name": "db", "image": "db:1"}]}}},
    }))
    .unwrap()
}

#[test]
fn deployments_and_statefulsets_are_all_stamped_with_the_restart_time() {
    let t = Selected::new(false);
    t.port().insert(deployment("web"));
    t.port().insert(deployment("api"));
    t.port().insert(statefulset("db"));
    let targets = vec![
        deployment_ref("web"),
        ns_ref("StatefulSet", "apps", "shop", "db"),
        deployment_ref("api"),
    ];
    let out = t.run(restart_of(targets), Initiator::Ui).expect("restart");
    assert_eq!(out.message.as_deref(), Some("Restarted 3 objects"));
    let report = BulkReport::from_data(out.data.as_ref()).unwrap();
    assert!(
        report
            .rows
            .iter()
            .all(|r| r.status.message() == "Restarting")
    );
    for (kind, name) in [
        ("Deployment", "web"),
        ("Deployment", "api"),
        ("StatefulSet", "db"),
    ] {
        let live = t.object(kind, name).unwrap();
        let stamp = live.to_value()["spec"]["template"]["metadata"]["annotations"][RESTARTED_AT]
            .as_str()
            .map(str::to_owned);
        assert!(stamp.is_some(), "{kind} {name} was stamped");
    }
    // Each: the server's rehearsal, then the patch.
    assert_eq!(t.writes().len(), 6);
    assert!(t.writes().iter().filter(|(_, _, dry)| *dry).count() == 3);
}

#[test]
fn one_non_rollable_object_refuses_the_whole_batch_before_a_request() {
    let t = Selected::new(false);
    t.port().insert(deployment("web"));
    t.port().insert(pod("p", "default", json!({})));
    for odd in [pod_ref("p"), ns_ref("ReplicaSet", "apps", "shop", "web-5d")] {
        let error = OxiError::from(
            t.run(restart_of(vec![deployment_ref("web"), odd]), Initiator::Ui)
                .unwrap_err(),
        );
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert!(
            error
                .message()
                .contains("only Deployments, StatefulSets and DaemonSets"),
            "{}",
            error.message()
        );
    }
    assert!(
        t.writes().is_empty(),
        "the Deployment was not restarted either"
    );
}

#[test]
fn a_workload_that_is_gone_is_skipped_and_the_rest_restart() {
    let t = Selected::new(false);
    t.port().insert(deployment("web"));
    let out = t
        .run(
            restart_of(vec![deployment_ref("ghost"), deployment_ref("web")]),
            Initiator::Ui,
        )
        .unwrap();
    let report = BulkReport::from_data(out.data.as_ref()).unwrap();
    let status: Vec<_> = report.rows.iter().map(|r| r.status.label()).collect();
    assert_eq!(status, ["Skipped", "Done"]);
}

#[test]
fn each_workload_is_audited_with_the_batch_id_and_what_the_server_answered() {
    let t = Selected::new(false);
    t.port().insert(deployment("web"));
    t.port().insert(deployment("api"));
    let out = t
        .run(
            restart_of(vec![deployment_ref("web"), deployment_ref("api")]),
            Initiator::Ui,
        )
        .unwrap();
    let batch = BulkReport::from_data(out.data.as_ref()).unwrap().batch;
    let records: Vec<_> =
        t.h.audit()
            .into_iter()
            .filter(|r| r.cmd.as_ref() == "workload::Restart")
            .collect();
    assert_eq!(records.len(), 2);
    for record in records {
        assert_eq!(record.outcome, AuditOutcome::Succeeded);
        let detail = record.detail.as_deref().unwrap();
        assert!(
            detail.starts_with(&format!(
                "batch={batch} via=selection::Restart restartedAt="
            )),
            "{detail}"
        );
    }
}
