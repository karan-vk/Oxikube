//! `workload::SetImage`: a strategic merge patch naming only the changed containers.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;
use serde_json::{Value, json};

use super::{deployment, rollable, set_image_of, workload};
use crate::command_bus::DispatchError;
use crate::testing::{Harness, ctx};
use crate::workloads::{SetImageRunner, WorkloadService};

fn patches(h: &Harness) -> Vec<(bool, PatchKind, Value)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch { patch, options, .. } => (options.dry_run, patch.kind, patch.body),
            other => panic!("expected a patch, got {other:?}"),
        })
        .collect()
}

fn with_deployment() -> Harness {
    let h = Harness::with_workloads();
    h.connect("a", false);
    h.resources("a").insert(rollable("Deployment", "web", None));
    h
}

fn handler_error(error: DispatchError) -> oxikube_domain::OxiError {
    match error {
        DispatchError::Handler(error) => error,
        other => panic!("expected a handler error, got {other:?}"),
    }
}

#[test]
fn set_image_sends_a_strategic_patch_with_only_the_named_container() {
    let h = with_deployment();
    let output = h
        .confirm_and_run(
            set_image_of(deployment("a"), &[("web", "nginx:1.27")]),
            ctx(Initiator::Ui),
        )
        .unwrap()
        .completed()
        .unwrap();
    assert_eq!(
        output.message.as_deref(),
        Some(
            "Set the image of Deployment shop/web (web: nginx:1.26 -> nginx:1.27); \
             new pods are rolling out"
        )
    );
    let body = json!({"spec": {"template": {"spec": {
        "containers": [{"name": "web", "image": "nginx:1.27"}]
    }}}});
    // The guard's dry run before it asked (E19-S07), the handler's, the patch.
    assert_eq!(
        patches(&h),
        [
            (true, PatchKind::Strategic, body.clone()),
            (true, PatchKind::Strategic, body.clone()),
            (false, PatchKind::Strategic, body)
        ]
    );
    // The stored object shows what the server does with it: the one image changed, the rest of
    // the template (the other containers, the web container's ports) is as it was.
    let live = &h.resources("a").objects()[0];
    assert_eq!(
        live.get_str("/spec/template/spec/containers/0/image"),
        Some("nginx:1.27")
    );
    assert_eq!(
        live.get_i64("/spec/template/spec/containers/0/ports/0/containerPort"),
        Some(80)
    );
    assert_eq!(
        live.get_str("/spec/template/spec/containers/1/image"),
        Some("envoy:1")
    );
    assert_eq!(
        live.get_str("/spec/template/spec/initContainers/0/image"),
        Some("app:1")
    );
}

#[test]
fn an_init_container_goes_in_its_own_list() {
    let h = with_deployment();
    h.confirm_and_run(
        set_image_of(deployment("a"), &[("migrate", "app:2")]),
        ctx(Initiator::Ui),
    )
    .unwrap();
    assert_eq!(
        patches(&h)[1].2,
        json!({"spec": {"template": {"spec": {
            "initContainers": [{"name": "migrate", "image": "app:2"}]
        }}}})
    );
    let live = &h.resources("a").objects()[0];
    assert_eq!(
        live.get_str("/spec/template/spec/initContainers/0/image"),
        Some("app:2")
    );
    assert_eq!(
        live.get_str("/spec/template/spec/containers/0/image"),
        Some("nginx:1.26")
    );
}

#[test]
fn several_containers_change_in_one_patch() {
    let h = with_deployment();
    h.confirm_and_run(
        set_image_of(
            deployment("a"),
            &[
                ("web", "nginx:1.27"),
                ("sidecar", "envoy:2"),
                ("migrate", "app:2"),
            ],
        ),
        ctx(Initiator::Ui),
    )
    .unwrap();
    assert_eq!(
        patches(&h).len(),
        3,
        "the guard's dry run, one rehearsal and one patch, not one per container"
    );
    assert_eq!(
        patches(&h)[2].2,
        json!({"spec": {"template": {"spec": {
            "initContainers": [{"name": "migrate", "image": "app:2"}],
            "containers": [
                {"name": "web", "image": "nginx:1.27"},
                {"name": "sidecar", "image": "envoy:2"},
            ],
        }}}})
    );
}

#[test]
fn deployments_statefulsets_and_daemonsets_take_it_and_the_others_are_refused() {
    for kind in ["Deployment", "StatefulSet", "DaemonSet"] {
        let h = Harness::with_workloads();
        h.connect("a", false);
        h.resources("a").insert(rollable(kind, "x", None));
        h.confirm_and_run(
            set_image_of(workload("a", "apps", kind, "x"), &[("web", "nginx:9")]),
            ctx(Initiator::Ui),
        )
        .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let live = &h.resources("a").objects()[0];
        assert_eq!(
            live.get_str("/spec/template/spec/containers/0/image"),
            Some("nginx:9"),
            "{kind}"
        );
    }
    for (group, kind) in [
        ("apps", "ReplicaSet"),
        ("", "ReplicationController"),
        ("", "Pod"),
    ] {
        let h = Harness::with_workloads();
        h.connect("a", false);
        let error = h
            .confirm_and_run(
                set_image_of(workload("a", group, kind, "x"), &[("web", "nginx:9")]),
                ctx(Initiator::Ui),
            )
            .unwrap_err();
        let error = handler_error(error);
        assert_eq!(error.kind(), ErrorKind::Validation, "{kind}");
        assert!(
            error.message().contains("cannot have its image set"),
            "{kind}"
        );
        assert!(h.writes("a").is_empty(), "{kind}");
    }
}

#[test]
fn bad_input_is_refused_before_any_request() {
    for (images, needle) in [
        (vec![], "Name a container"),
        (vec![("web", "")], "Enter an image"),
        (vec![("web", "  ")], "Enter an image"),
        (vec![("web", "nginx 1")], "cannot contain spaces"),
        (vec![("ghost", "x:1")], "no container named ghost"),
        (
            vec![("web", "nginx:1.26")],
            "what the containers run already",
        ),
    ] {
        let h = with_deployment();
        let error = h
            .confirm_and_run(set_image_of(deployment("a"), &images), ctx(Initiator::Ui))
            .unwrap_err();
        let error = handler_error(error);
        assert_eq!(error.kind(), ErrorKind::Validation, "{images:?}");
        assert!(
            error.message().contains(needle),
            "{images:?}: {}",
            error.message()
        );
        assert!(h.writes("a").is_empty(), "{images:?}");
        assert_eq!(h.audit()[0].outcome, AuditOutcome::Failed, "{images:?}");
    }
}

#[test]
fn set_image_is_low_risk_with_one_simple_confirmation_that_names_the_images() {
    let h = with_deployment();
    for initiator in Initiator::ALL {
        let request = h.ask(
            set_image_of(deployment("a"), &[("web", "nginx:1.27")]),
            ctx(initiator),
        );
        assert_eq!(request.risk, Some(Risk::Low), "{initiator}");
        assert_eq!(request.tier, ConfirmTier::Simple, "{initiator}");
        assert!(
            request
                .summary
                .starts_with("Set the image of Deployment shop/web on "),
            "{}",
            request.summary
        );
        assert!(
            request.summary.contains("web to nginx:1.27"),
            "{}",
            request.summary
        );
        let Some(crate::guard::DryRunPreview::Rehearsed(preview)) = &request.preview else {
            panic!("a patch is previewed: {request:?}");
        };
        assert_eq!(
            preview.changes[0].result["spec"]["template"]["spec"]["containers"][0]["image"],
            "nginx:1.27"
        );
        assert_eq!(
            preview.changes[0].live.as_deref().unwrap()["spec"]["template"]["spec"]["containers"]
                [0]["image"],
            "nginx:1.26"
        );
    }
    // Only the guard's dry runs before the answer.
    assert!(h.writes("a").iter().all(ResourceCall::is_dry_run));
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_before_any_request() {
    let h = Harness::with_workloads();
    h.connect("a", true);
    for initiator in Initiator::ALL {
        let error = h
            .dispatch(
                set_image_of(deployment("a"), &[("web", "x:2")]),
                ctx(initiator),
            )
            .unwrap_err();
        assert!(
            matches!(error, DispatchError::ReadOnly { .. }),
            "{initiator}"
        );
    }
    assert!(
        h.resources("a").recorded_calls().is_empty(),
        "no request of any kind"
    );
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn the_audit_record_has_the_old_and_new_image_and_the_servers_version() {
    let h = with_deployment();
    h.confirm_and_run(
        set_image_of(
            deployment("a"),
            &[("web", "nginx:1.27"), ("migrate", "app:2")],
        ),
        ctx(Initiator::Agent),
    )
    .unwrap();
    let record = &h.audit()[0];
    assert_eq!(&*record.cmd, "workload::SetImage");
    assert_eq!(record.target, deployment("a"));
    assert_eq!(record.initiator, Initiator::Agent);
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(
        record.detail.as_deref(),
        Some("containers=web,migrate web=nginx:1.26->nginx:1.27,migrate=app:1->app:2 rv=7")
    );
}

#[test]
fn a_failed_set_image_is_audited_with_the_containers_it_asked_for() {
    let h = with_deployment();
    h.resources("a")
        .script()
        .patch
        .push_err(oxikube_domain::OxiError::forbidden("denied by admission"));
    h.confirm_and_run(
        set_image_of(deployment("a"), &[("web", "nginx:1.27")]),
        ctx(Initiator::Ui),
    )
    .unwrap_err();
    let record = &h.audit()[0];
    assert_eq!(record.outcome, AuditOutcome::Failed);
    assert_eq!(record.detail.as_deref(), Some("containers=web"));
    assert_eq!(
        patches(&h).len(),
        1,
        "the rehearsal was refused: no real patch"
    );
}

#[test]
fn a_dry_run_dispatch_rehearses_once_and_changes_nothing() {
    let h = with_deployment();
    let output = h
        .confirm_and_run(
            set_image_of(deployment("a"), &[("web", "nginx:1.27")]),
            ctx(Initiator::Agent).with_dry_run(true),
        )
        .unwrap()
        .completed()
        .unwrap();
    assert!(
        output
            .message
            .unwrap()
            .starts_with("Dry run: the image of Deployment shop/web")
    );
    let sent = patches(&h);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].0);
    assert_eq!(
        h.resources("a").objects()[0].get_str("/spec/template/spec/containers/0/image"),
        Some("nginx:1.26")
    );
}

#[test]
fn the_plan_lists_init_containers_first_with_their_images() {
    let h = with_deployment();
    let service = WorkloadService::new(h.manager.clone());
    let plan = futures::executor::block_on(service.plan_set_image(&deployment("a"))).unwrap();
    let listed: Vec<_> = plan
        .containers
        .iter()
        .map(|c| (&*c.name, &*c.image, c.init))
        .collect();
    assert_eq!(
        listed,
        [
            ("migrate", "app:1", true),
            ("web", "nginx:1.26", false),
            ("sidecar", "envoy:1", false)
        ]
    );
    // Reading is not a mutation: nothing was written, nothing audited.
    assert!(h.writes("a").is_empty());
    assert!(h.audit().is_empty());
    let gone = futures::executor::block_on(service.plan_set_image(&workload(
        "a",
        "apps",
        "Deployment",
        "gone",
    )));
    assert_eq!(gone.unwrap_err().kind(), ErrorKind::NotFound);
    let rs = futures::executor::block_on(service.plan_set_image(&workload(
        "a",
        "apps",
        "ReplicaSet",
        "x",
    )));
    assert_eq!(rs.unwrap_err().kind(), ErrorKind::Validation);
}

#[test]
fn the_runner_answers_the_simple_confirmation_and_reports() {
    let h = with_deployment();
    let runner = SetImageRunner::new(h.bus.clone(), "alice");
    let report = futures::executor::block_on(runner.run(
        &deployment("a"),
        vec![oxikube_domain::command::ImageChange::new(
            "web",
            "nginx:1.27",
        )],
    ))
    .unwrap();
    assert!(
        report
            .message
            .starts_with("Set the image of Deployment shop/web")
    );
    let audit = h.audit();
    assert_eq!(audit.len(), 2, "the guard's dry run, then the patch");
    assert!(audit[0].dry_run && !audit[1].dry_run);
    assert_eq!(audit[1].initiator, Initiator::Ui);
    assert_eq!(&*audit[1].who, "alice");

    // A read-only cluster refuses the runner too.
    let ro = Harness::with_workloads();
    ro.connect("a", true);
    let runner = SetImageRunner::new(ro.bus.clone(), "alice");
    let error = futures::executor::block_on(runner.run(
        &deployment("a"),
        vec![oxikube_domain::command::ImageChange::new(
            "web",
            "nginx:1.27",
        )],
    ))
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(ro.resources("a").recorded_calls().is_empty());
}
