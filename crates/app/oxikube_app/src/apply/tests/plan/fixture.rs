//! The 50-manifest fixture directory (E20-S10) through the real guard and handlers over the fakes:
//! the same scenarios the kind suite runs, without a cluster. The fixture's file order is the
//! opposite of the right apply order, so a plan that applied in file order would fail here.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt as _;
use oxikube_domain::Resource;
use oxikube_domain::apply::kind_rank;
use oxikube_domain::ids::Gvk;
use oxikube_domain::manifests::ManifestDoc;
use oxikube_ports::FsPort;
use oxikube_testkit::fixtures::apply as fixture;
use oxikube_testkit::{FakeFsPort, ResourceCall};

use super::*;
use crate::apply::{LoadLimits, load_path};

const RUN: &str = "oxi-test-fx01";

fn rig(read_only: bool) -> Applying {
    let t = Applying::new(read_only);
    let mut kinds = oxikube_testkit::kinds::core_kinds();
    kinds.extend(fixture::served_kinds(RUN));
    t.h.connector.ports_for(&id("a")).discovery.set_kinds(kinds);
    t
}

fn plan_of(t: &Applying) -> ClusterPlan {
    let target = PlanTarget {
        cluster: id("a"),
        namespace: None,
    };
    t.service
        .plan(fixture::docs(RUN), target)
        .now_or_never()
        .expect("the fakes answer at once")
}

fn kind_and_name(call: &ResourceCall) -> (String, String) {
    let body = super::super::body(call);
    (
        body["kind"].as_str().unwrap().to_owned(),
        body["metadata"]["name"].as_str().unwrap().to_owned(),
    )
}

fn gvk(doc: &ManifestDoc) -> Gvk {
    Gvk::from_api_version(
        doc.json["apiVersion"].as_str().unwrap(),
        doc.json["kind"].as_str().unwrap(),
    )
}

#[test]
fn the_plan_holds_all_fifty_in_kind_order_not_file_order() {
    let t = rig(false);
    let plan = plan_of(&t);
    let items = &plan.plan.items;
    assert_eq!(items.len(), fixture::MANIFEST_COUNT);
    let ranks: Vec<u8> = items.iter().map(|i| kind_rank(&gvk(&i.doc))).collect();
    assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
    let kinds: Vec<&str> = items
        .iter()
        .map(|i| i.doc.json["kind"].as_str().unwrap())
        .collect();
    assert_eq!(&kinds[..2], ["Namespace", "Namespace"]);
    let crd = kinds.iter().position(|k| *k == "CustomResourceDefinition");
    let first_gadget = kinds.iter().position(|k| *k == "Gadget");
    assert!(crd < first_gadget, "the definition before its resources");
    // The first file of the directory holds Gadgets: they moved to the back.
    assert_eq!(items.last().unwrap().doc.json["kind"], "Gadget");
    // Every namespaced object has a namespace, and keeps its file and line.
    for item in items {
        assert!(
            item.doc.json["metadata"].get("uid").is_none(),
            "sanitised for apply"
        );
        assert!(item.doc.line >= 1 && !item.doc.source.to_string().is_empty());
    }
    assert!(t.calls().is_empty(), "planning only reads discovery");
}

#[test]
fn applying_the_directory_writes_in_plan_order_after_a_full_dry_run() {
    let t = rig(false);
    let plan = plan_of(&t);
    let mut pool = LocalPool::new();
    let done = pool
        .spawner()
        .spawn_local_with_handle(
            t.service
                .apply_plan(&plan, &ApplyOptions::default(), Answering::confirm())
                .into_report(),
        )
        .unwrap();
    pool.run_until_stalled();
    // The definition is written and polled until the API server says Established.
    let crd_name = format!("gadgets.{}", fixture::crd_group(RUN));
    let applied = t.applied_names();
    assert_eq!(applied.last().map(String::as_str), Some(crd_name.as_str()));
    assert!(
        !applied.iter().any(|n| n.starts_with("gadget-")),
        "no custom resource before the definition is Established: {applied:?}"
    );
    let mut definition = plan
        .plan
        .items
        .iter()
        .find(|i| i.doc.json["kind"] == "CustomResourceDefinition")
        .unwrap()
        .doc
        .json
        .clone();
    definition["status"] = json!({"conditions": [{"type": "Established", "status": "True"}]});
    t.port().insert(Resource::from_json(definition).unwrap());
    t.h.clock.advance(Duration::from_millis(500));
    pool.run_until_stalled();
    let report = pool.run_until(done).expect("applied");

    assert_eq!(report.objects.len(), fixture::MANIFEST_COUNT);
    assert!(
        report.objects.iter().all(|o| o.status.word() == "created"),
        "{}",
        report.summary()
    );
    let writes = t.h.writes("a");
    let first_real = writes.iter().position(|c| !c.is_dry_run()).unwrap();
    // Every object with no prerequisite in the plan is rehearsed before the first real write;
    // the others are rehearsed once what they need was applied.
    let independent = plan
        .plan
        .items
        .iter()
        .filter(|i| i.depends_on.is_empty())
        .count();
    assert!(
        writes[..first_real].iter().all(ResourceCall::is_dry_run) && first_real >= independent,
        "{first_real} rehearsals before the first real write, {independent} independent objects"
    );
    let real: Vec<(String, String)> = writes
        .iter()
        .filter(|c| !c.is_dry_run())
        .map(kind_and_name)
        .collect();
    let planned: Vec<(String, String)> = plan
        .plan
        .items
        .iter()
        .map(|i| {
            (
                i.doc.json["kind"].as_str().unwrap().to_owned(),
                i.doc.json["metadata"]["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(real, planned, "written exactly in plan order");
    assert!(
        t.h.audit()
            .iter()
            .filter(|r| r.cmd.as_ref() == "resource::Apply" && !r.dry_run)
            .count()
            >= fixture::MANIFEST_COUNT
    );
}

#[test]
fn a_read_only_cluster_refuses_the_whole_directory_with_zero_calls() {
    let t = rig(true);
    let plan = plan_of(&t);
    let confirm = Answering::confirm();
    let error = t
        .apply_plan(&plan, &ApplyOptions::default(), confirm.clone())
        .unwrap_err();
    assert!(matches!(error, ApplyError::ReadOnly { .. }), "{error:?}");
    assert!(t.calls().is_empty(), "not even a read: {:?}", t.calls());
    assert!(confirm.asked.lock().is_empty());
}

#[test]
fn secret_values_do_not_reach_the_debug_form_of_the_plan() {
    let t = rig(false);
    let plan = plan_of(&t);
    let text = format!("{plan:?}");
    assert!(!text.contains("dummy-password"), "{text}");
    assert!(!text.contains("not-a-real-token"), "{text}");
}

#[test]
fn the_directory_loads_through_the_file_system_port_in_path_order() {
    let mut fake = FakeFsPort::new();
    for file in fixture::files(RUN) {
        fake = fake.with_file(Path::new("/apply").join(&file.path), file.text);
    }
    // Not a manifest: the loader skips it.
    fake = fake.with_file("/apply/99-namespaces/NOTES.txt", "notes");
    let fs: Arc<dyn FsPort> = Arc::new(fake);
    let loaded =
        now_or_never(load_path(&fs, Path::new("/apply"), LoadLimits::default())).expect("loads");
    assert_eq!(loaded.docs.len(), fixture::MANIFEST_COUNT);
    assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
    let direct: Vec<_> = fixture::docs(RUN)
        .iter()
        .map(|d| (d.source.to_string(), d.line, d.json.clone()))
        .collect();
    let via_loader: Vec<_> = loaded
        .docs
        .iter()
        .map(|d| {
            let source = d.source.to_string();
            let relative = source.strip_prefix("/apply/").unwrap_or(&source).to_owned();
            (relative, d.line, d.json.clone())
        })
        .collect();
    assert_eq!(
        via_loader, direct,
        "the S01 loader sees what the fixture says"
    );
}

fn now_or_never<T>(future: impl std::future::Future<Output = T>) -> T {
    futures::FutureExt::now_or_never(future).expect("the fakes answer at once")
}
