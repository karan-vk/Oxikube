//! The plan and its two passes: kind order, the whole plan dry-run before the first write, every
//! object server-side applied under `oxikube`, results streamed and aggregated per object.

use futures::StreamExt as _;
use oxikube_domain::OxiError;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_ports::PatchKind;
use oxikube_testkit::ResourceCall;

use super::*;
use crate::apply::{ApplyMethod, MAX_PLAN_OBJECTS};
use crate::command_bus::{DispatchContext, Outcome};

#[test]
fn the_plan_orders_by_kind_defaults_the_namespace_and_sanitises() {
    let t = Applying::new(false);
    let mut with_status = deployment("web");
    with_status["status"] = json!({"replicas": 3});
    with_status["metadata"]["resourceVersion"] = json!("41");
    let plan = t.plan_in(
        vec![with_status, configmap("settings", None), namespace("shop")],
        Some("shop"),
    );
    let kinds: Vec<&str> = plan
        .plan
        .items
        .iter()
        .map(|item| item.doc.json["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["Namespace", "ConfigMap", "Deployment"]);
    let deployment = &plan.plan.items[2].doc;
    assert_eq!(deployment.json["metadata"]["namespace"], "shop");
    assert!(deployment.json.get("status").is_none(), "sanitised");
    assert!(deployment.json["metadata"].get("resourceVersion").is_none());
    assert_eq!(
        (deployment.source.to_string(), deployment.line),
        ("dir/app.yaml".to_owned(), 1),
        "the document keeps where it came from (open in editor)"
    );
    assert!(
        plan.plan.items[0].doc.json["metadata"]
            .get("namespace")
            .is_none()
    );
    assert_eq!(
        plan.plan.items[1].depends_on,
        [0],
        "the ConfigMap waits for shop"
    );
    assert!(t.calls().is_empty(), "planning reads discovery only");
}

#[test]
fn every_object_is_dry_run_before_the_first_write_then_applied_in_kind_order() {
    let t = Applying::new(false);
    // Given out of order: the plan puts the namespace first and the deployment last.
    let plan = t.plan_in(
        vec![
            deployment("web"),
            configmap("settings", None),
            namespace("shop"),
        ],
        Some("shop"),
    );
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["created", "created", "created"]);
    assert_eq!(
        labels(&report),
        [
            "Namespace shop",
            "ConfigMap shop/settings",
            "Deployment shop/web"
        ]
    );
    assert_eq!(t.applied_names(), ["shop", "settings", "web"]);
    let writes = t.writes();
    let first_real = writes.iter().position(|(_, dry_run)| !dry_run).unwrap();
    assert_eq!(
        first_real, 4,
        "three dry runs, then the namespace's own rehearsal"
    );
    assert!(writes[..3].iter().all(|(_, dry_run)| *dry_run));
}

#[test]
fn each_object_goes_by_server_side_apply_under_oxikube() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None), configmap("b", None)]);
    t.apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    let patches: Vec<ResourceCall> = t.h.writes("a");
    assert_eq!(
        patches.len(),
        6,
        "a plan dry run, then a dry run and an apply, each"
    );
    for call in patches {
        let ResourceCall::Patch { patch, options, .. } = call else {
            panic!("only patches: {call:?}");
        };
        assert_eq!(
            patch.kind,
            PatchKind::Apply {
                manager: "oxikube".into(),
                force: false
            }
        );
        assert_eq!(options.field_manager.as_deref(), Some("oxikube"));
    }
}

#[test]
fn another_field_manager_is_used_when_asked() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None)]);
    let options = ApplyOptions {
        field_manager: Some("team-a".into()),
        ..ApplyOptions::default()
    };
    t.apply_plan(&plan, &options, Answering::confirm())
        .expect("applied");
    assert!(t.h.writes("a").iter().all(|call| matches!(
        call,
        ResourceCall::Patch { options, .. } if options.field_manager.as_deref() == Some("team-a")
    )));
}

#[test]
fn the_dry_run_writes_nothing_and_says_what_each_object_would_do() {
    let t = Applying::new(false);
    t.port().insert(super::super::live_configmap("5"));
    let mut same = configmap("same", Some("default"));
    same["data"]["mode"] = json!("green");
    t.port().insert(
        oxikube_domain::Resource::from_json({
            let mut live = same.clone();
            live["metadata"]["resourceVersion"] = json!("9");
            live
        })
        .unwrap(),
    );
    let plan = t.plan(vec![
        configmap("new", None),
        configmap("web", None), // live has mode green, this says blue
        same,
    ]);
    let report = t
        .dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    assert_eq!(words(&report), ["created", "configured", "unchanged"]);
    assert!(report.objects.iter().all(|o| o.dry_run));
    assert!(
        t.writes().iter().all(|(_, dry_run)| *dry_run),
        "nothing written"
    );
    assert_eq!(report.summary(), "1 created, 1 configured, 1 unchanged");
}

#[test]
fn results_stream_one_per_object_as_they_are_decided() {
    let t = Applying::new(false);
    let plan = t.plan(vec![
        configmap("a", None),
        configmap("b", None),
        namespace("x"),
    ]);
    let stream = t
        .service
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm());
    let items: Vec<_> = stream
        .collect::<Vec<_>>()
        .now_or_never()
        .expect("the fakes answer at once");
    assert_eq!(items.len(), 3, "one item per object, no terminal error");
    let indices: Vec<usize> = items.iter().map(|i| i.as_ref().unwrap().index).collect();
    assert_eq!(indices, [0, 1, 2], "ranks in order");
}

#[test]
fn an_agent_dispatch_without_progress_gets_every_result_in_the_output() {
    let t = Applying::new(false);
    let plan = t.plan(vec![configmap("a", None), namespace("x")]);
    let command = Command::ResourceApplyPlan {
        cluster: id("a"),
        namespace: None,
        objects: plan
            .plan
            .items
            .iter()
            .map(|i| i.doc.json.clone())
            .collect::<Vec<_>>()
            .into(),
        field_manager: None,
        force_conflicts: false,
        stop_on_error: false,
    };
    let outcome =
        t.h.confirm_and_run(command, DispatchContext::new(Initiator::Agent, "agent"))
            .expect("ran");
    let Outcome::Completed(output) = outcome else {
        panic!("completed");
    };
    let data = output.data.expect("data");
    let objects: Vec<ObjectResult> = serde_json::from_value(data["objects"].clone()).unwrap();
    assert_eq!(objects.len(), 2);
    assert_eq!(output.message.as_deref(), Some("2 created"));
    assert!(data["batch"].as_str().unwrap().starts_with("apply-"));
}

#[test]
fn an_empty_or_oversized_plan_is_refused_before_any_request() {
    let t = Applying::new(false);
    let empty = t.plan(Vec::new());
    let error = t
        .apply_plan(&empty, &ApplyOptions::default(), Answering::confirm())
        .unwrap_err();
    assert!(matches!(error, ApplyError::Failed(_)), "{error:?}");
    let many = t.plan(vec![configmap("a", None); MAX_PLAN_OBJECTS + 1]);
    let error = t
        .apply_plan(&many, &ApplyOptions::default(), Answering::confirm())
        .unwrap_err();
    assert!(error.to_string().contains("at most"), "{error}");
    assert!(t.calls().is_empty());
}

#[test]
fn an_object_that_names_nothing_fails_alone() {
    let t = Applying::new(false);
    let nameless = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {}});
    let plan = t.plan(vec![nameless, configmap("ok", None)]);
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["failed", "created"]);
    assert_eq!(report.objects[0].label, "Object 1");
    assert!(reason(&report.objects[0]).contains("metadata.name"));
    assert_eq!(t.applied_names(), ["ok"]);
}

#[test]
fn a_kind_the_server_cannot_dry_run_has_no_preview() {
    let t = Applying::new(false);
    // Neither the server-side apply nor the fallback's create can be rehearsed.
    t.port()
        .script()
        .patch
        .push_err(OxiError::unsupported("dry run is not supported"));
    t.port()
        .script()
        .create
        .push_err(OxiError::unsupported("dry run is not supported"));
    let plan = t.plan(vec![configmap("a", None), configmap("b", None)]);
    let report = t
        .dry_run_plan(&plan, &ApplyOptions::default())
        .expect("dry run");
    assert_eq!(
        words(&report),
        ["no preview", "created"],
        "the plan goes on"
    );
}

#[test]
fn an_object_without_a_preview_is_applied_without_another_dry_run() {
    let t = Applying::new(false);
    let unsupported = || OxiError::unsupported("dry run is not supported");
    // The rehearsal: neither dry run. The apply: no server-side apply either, so it is created.
    t.port().script().patch.push_err(unsupported());
    t.port().script().patch.push_err(unsupported());
    t.port().script().create.push_err(unsupported());
    let plan = t.plan(vec![configmap("a", None)]);
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["created"], "applied without a preview");
    assert_eq!(report.objects[0].method, Some(ApplyMethod::Create));
    assert_eq!(
        t.writes(),
        [
            ("patch", true),
            ("create", true),
            ("patch", false),
            ("create", false)
        ],
        "the server is not asked for a dry run it cannot do"
    );
}

#[test]
fn an_object_without_a_preview_is_still_server_side_applied_when_the_server_takes_it() {
    let t = Applying::new(false);
    // Only the dry runs are unsupported.
    t.port()
        .script()
        .patch
        .push_err(OxiError::unsupported("dry run is not supported"));
    t.port()
        .script()
        .create
        .push_err(OxiError::unsupported("dry run is not supported"));
    let plan = t.plan(vec![configmap("a", None)]);
    let report = t
        .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
        .expect("applied");
    assert_eq!(words(&report), ["created"]);
    assert_eq!(report.objects[0].method, Some(ApplyMethod::ServerSide));
    assert_eq!(
        t.writes(),
        [("patch", true), ("create", true), ("patch", false)],
        "one server-side apply, no dry run"
    );
}
