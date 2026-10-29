//! The 50-manifest apply fixture (E20-S10, `oxikube_testkit::fixtures::apply`) against a real
//! API server: `ApplyService::plan` / `apply_plan` through the `CommandBus` and `MutationGuard`,
//! and `CopyService` for objects the apply made. No UI.
//!
//! * [`the_directory_applies_in_kind_order_and_is_idempotent`]: the files are given in the wrong
//!   order and from an empty cluster, so the CustomResourceDefinition must be `Established`
//!   before its custom resources (no "no matches for kind"); every object is `created`, in plan
//!   order (the audit log's order; `metadata.resourceVersion` is not used because the API server
//!   keeps bumping it: the CustomResourceDefinition gets its status, Deployments theirs); the
//!   same plan again is all `unchanged`; one changed value is exactly one `configured`; a value
//!   another field manager took is a conflict naming that manager, until forced; deleting the
//!   namespaces and applying again `created` everything in them.
//! * [`a_deployment_and_a_configmap_are_copied_to_the_other_namespace`]: the copies are objects
//!   `kubectl apply --dry-run=server` accepts, without the server's fields.
//! * [`copy_to_a_second_cluster`]: the same across two real clusters, only with
//!   `OXIKUBE_TEST_CONTEXT_B` set (the nightly job: `cargo xtask kind-up --name oxikube-b`).
//!
//! Every run has its own `oxi-test-<rand>` id in its namespaces, group and cluster-scoped names;
//! all of it is deleted when the test ends, also when it panics. The Secrets hold dummy values;
//! nothing here prints an object's data.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use oxikube_app::apply::{ApplyOptions, ObjectStatus, PlanReport, conflict_managers};
use oxikube_app::copy::{CopyOutcome, ReviewAnswer};
use oxikube_domain::apply::kind_rank;
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::manifests::ManifestDoc;
use oxikube_testkit::fixtures::apply as fixture;
use oxikube_testkit::integration::random_namespace_name;
use serde_json::{Value, json};

use crate::apply_plan::{Rig as ApplyRig, Yes};
use crate::cluster::Kind;
use crate::copy::Rig as CopyRig;

/// What a run leaves in the cluster, deleted (no wait) when dropped.
struct Cleanup {
    context: String,
    run: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let mut targets: Vec<(&str, String)> = fixture::namespaces(&self.run)
            .into_iter()
            .map(|name| ("namespace", name))
            .collect();
        targets.extend(fixture::cluster_scoped(&self.run));
        for (kind, name) in targets {
            let _ = Command::new("kubectl")
                .args(["--context", &self.context, "delete", kind, &name])
                .args(["--ignore-not-found", "--wait=false"])
                .output();
        }
    }
}

/// What a run did, as lines of names and status words: never an object's data.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn add(&self, step: &str, report: &PlanReport) {
        let mut lines = self.0.lock().expect("log lock");
        lines.push(format!("== {step}: {}", report.summary()));
        for object in &report.objects {
            let detail = match &object.status {
                ObjectStatus::Failed { reason, .. } | ObjectStatus::Skipped { reason } => {
                    format!(" ({reason})")
                }
                _ => String::new(),
            };
            lines.push(format!("{} {}{detail}", object.status.word(), object.label));
        }
    }
}

/// When the test panics, saves the run's results and the manifests that hold no Secret under
/// `$OXIKUBE_TEST_DIAGNOSTICS_DIR/apply/<run>/` (the CI job uploads that directory).
struct Diagnostics {
    run: String,
    log: Log,
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        let Some(root) = std::env::var_os(oxikube_testkit::integration::DIAGNOSTICS_DIR_ENV) else {
            return;
        };
        if !std::thread::panicking() {
            return;
        }
        let dir = std::path::Path::new(&root).join("apply").join(&self.run);
        let _ = std::fs::create_dir_all(&dir);
        let lines = self.log.0.lock().map(|l| l.join("\n")).unwrap_or_default();
        let _ = std::fs::write(dir.join("results.txt"), lines);
        for file in fixture::files(&self.run) {
            if file.text.contains("kind: Secret") {
                continue;
            }
            let path = dir.join(&file.path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, file.text);
        }
    }
}

/// `kubectl --context <context> <args>`, `stdin` piped in; its stdout, or its stderr as the error.
fn kubectl(context: &str, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut child = Command::new("kubectl")
        .args(["--context", context])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("kubectl runs");
    let mut input = child.stdin.take().expect("stdin");
    if let Some(text) = stdin {
        input.write_all(text.as_bytes()).expect("write to kubectl");
    }
    drop(input);
    let output = child.wait_with_output().expect("kubectl finishes");
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn get_json(context: &str, namespace: &str, what: &str) -> Value {
    let text = kubectl(context, &["-n", namespace, "get", what, "-o", "json"], None)
        .unwrap_or_else(|error| panic!("kubectl get {what}: {error}"));
    serde_json::from_str(&text).expect("kubectl prints JSON")
}

async fn apply_docs(
    rig: &ApplyRig,
    log: &Log,
    step: &str,
    docs: Vec<ManifestDoc>,
    options: &ApplyOptions,
) -> PlanReport {
    let plan = rig.service.plan(docs, rig.target.clone()).await;
    let report = rig
        .service
        .apply_plan(&plan, options, Arc::new(Yes))
        .into_report()
        .await
        .expect("the plan runs");
    log.add(step, &report);
    report
}

fn words(report: &PlanReport) -> String {
    report.summary()
}

fn object<'a>(report: &'a PlanReport, label_part: &str) -> &'a oxikube_app::apply::ObjectResult {
    report
        .objects
        .iter()
        .find(|o| o.label.contains(label_part))
        .unwrap_or_else(|| panic!("no object {label_part} in {}", words(report)))
}

#[tokio::test]
async fn the_directory_applies_in_kind_order_and_is_idempotent() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let context = kind.context.to_string();
    let run = random_namespace_name();
    let _cleanup = Cleanup {
        context: context.clone(),
        run: run.clone(),
    };
    let log = Log::default();
    let _diagnostics = Diagnostics {
        run: run.clone(),
        log: log.clone(),
    };
    let [app, data] = fixture::namespaces(&run);
    let rig = ApplyRig::new(&kind).await;
    let docs = fixture::docs(&run);
    assert_eq!(docs.len(), fixture::MANIFEST_COUNT);

    // The CRD race: from an empty cluster, in file order (Gadgets first, Namespaces last).
    let plan = rig.service.plan(docs.clone(), rig.target.clone()).await;
    let dry = rig
        .service
        .dry_run_plan(&plan, &ApplyOptions::default())
        .into_report()
        .await
        .expect("dry run");
    log.add("dry run", &dry);
    assert_eq!(dry.failures(), 0, "{}", words(&dry));
    let before = kubectl(&context, &["get", "namespace", &app, "-o", "name"], None);
    assert!(
        before.is_err_and(|e| e.contains("NotFound")),
        "a dry run creates nothing"
    );
    let report = rig
        .service
        .apply_plan(&plan, &ApplyOptions::default(), Arc::new(Yes))
        .into_report()
        .await
        .expect("applied");
    log.add("first apply", &report);
    assert_eq!(
        report.count("created"),
        fixture::MANIFEST_COUNT,
        "{}",
        words(&report)
    );
    assert_eq!(report.failures(), 0);

    // Written rank by rank (objects of one rank go out together, so their order among
    // themselves is not fixed): the audit log's order of the real records never goes back.
    let records = rig.state.audit_log();
    let written: Vec<&ResourceRef> = records
        .iter()
        .filter(|r| r.cmd.as_ref() == "resource::Apply" && !r.dry_run)
        .inspect(|r| assert_eq!(r.outcome, AuditOutcome::Succeeded))
        .map(|r| &r.target)
        .collect();
    let ranks: Vec<u8> = written.iter().map(|t| kind_rank(&t.gvk)).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "kind order, not file order: {ranks:?}"
    );
    let mut written_ids: Vec<String> = written
        .iter()
        .map(|t| format!("{} {}/{}", t.gvk.kind, t.namespace().unwrap_or(""), t.name))
        .collect();
    let mut planned_ids: Vec<String> = plan
        .plan
        .items
        .iter()
        .map(|i| {
            let meta = &i.doc.json["metadata"];
            format!(
                "{} {}/{}",
                i.doc.json["kind"].as_str().unwrap(),
                meta["namespace"].as_str().unwrap_or(""),
                meta["name"].as_str().unwrap()
            )
        })
        .collect();
    written_ids.sort();
    planned_ids.sort();
    assert_eq!(written_ids, planned_ids, "every object, once");
    assert_eq!(written[0].gvk.kind.as_ref(), "Namespace");
    let gadgets = get_json(
        &context,
        &app,
        &format!("gadgets.{}", fixture::crd_group(&run)),
    );
    assert_eq!(gadgets["items"].as_array().map(Vec::len), Some(5));

    // Again: nothing to do.
    let again = apply_docs(&rig, &log, "again", docs.clone(), &ApplyOptions::default()).await;
    assert_eq!(
        again.count("unchanged"),
        fixture::MANIFEST_COUNT,
        "{}",
        words(&again)
    );

    // One value changed: exactly that ConfigMap is configured.
    let mut changed = docs.clone();
    let settings = changed
        .iter_mut()
        .find(|d| d.json["kind"] == "ConfigMap" && d.json["metadata"]["name"] == "settings-2")
        .expect("settings-2");
    settings.json["data"]["FEATURE_2"] = json!("off");
    let after = apply_docs(
        &rig,
        &log,
        "one value changed",
        changed,
        &ApplyOptions::default(),
    )
    .await;
    assert_eq!(after.count("configured"), 1, "{}", words(&after));
    assert_eq!(after.count("unchanged"), fixture::MANIFEST_COUNT - 1);
    assert_eq!(
        object(&after, "settings-2").status,
        ObjectStatus::Configured
    );

    // A conflict: another manager takes a field oxikube applied.
    kubectl(
        &context,
        &[
            "-n",
            app.as_str(),
            "apply",
            "--server-side",
            "--force-conflicts",
            "--field-manager=other",
            "-f",
            "-",
        ],
        Some(&format!(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings-1\n  namespace: {app}\ndata:\n  LOG_LEVEL: debug\n"
        )),
    )
    .expect("another manager applies");
    let conflicted = apply_docs(
        &rig,
        &log,
        "conflict",
        docs.clone(),
        &ApplyOptions::default(),
    )
    .await;
    match &object(&conflicted, "settings-1").status {
        ObjectStatus::Failed { conflicts, .. } => {
            assert_eq!(conflict_managers(conflicts), ["other"], "{conflicted:#?}");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(conflicted.failures(), 1, "{}", words(&conflicted));
    let forced = ApplyOptions {
        force_conflicts: true,
        ..ApplyOptions::default()
    };
    let taken = apply_docs(&rig, &log, "forced", docs.clone(), &forced).await;
    assert_eq!(
        object(&taken, "settings-1").status,
        ObjectStatus::Configured
    );
    assert_eq!(taken.failures(), 0);

    // Deleting the namespaces and applying again: what lived in them is created, the cluster-wide
    // objects are still there.
    kubectl(
        &context,
        &[
            "delete",
            "namespace",
            app.as_str(),
            &data,
            "--wait=true",
            "--timeout=240s",
        ],
        None,
    )
    .expect("namespaces deleted");
    let recreated = apply_docs(
        &rig,
        &log,
        "namespaces recreated",
        docs,
        &ApplyOptions::default(),
    )
    .await;
    let cluster_wide = fixture::cluster_scoped(&run).len();
    assert_eq!(
        recreated.count("created"),
        fixture::MANIFEST_COUNT - cluster_wide,
        "{}",
        words(&recreated)
    );
    assert_eq!(recreated.count("unchanged"), cluster_wide);
    assert_eq!(recreated.failures(), 0);

    // Read-only: the whole plan is refused before any request.
    rig.manager
        .set_read_only(&rig.target.cluster, true)
        .expect("read-only");
    let plan = rig
        .service
        .plan(fixture::docs(&run), rig.target.clone())
        .await;
    let refused = rig
        .service
        .apply_plan(&plan, &ApplyOptions::default(), Arc::new(Yes))
        .into_report()
        .await;
    assert!(
        matches!(
            refused,
            Err(oxikube_app::apply::ApplyError::ReadOnly { .. })
        ),
        "{refused:?}"
    );
}

/// The fixture is valid Kubernetes: every manifest but the custom resources (whose kind `kubectl`
/// cannot map before the definition exists) passes `kubectl apply --dry-run=client`.
#[tokio::test]
async fn the_fixture_passes_kubectl_client_dry_run() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let dir = tempfile::tempdir().expect("temp dir");
    fixture::write_to(dir.path(), &random_namespace_name()).expect("write the fixture");
    std::fs::remove_dir_all(dir.path().join("00-custom-resources")).expect("custom resources");
    let path = dir.path().to_string_lossy().into_owned();
    let context = kind.context.to_string();
    let out = kubectl(
        &context,
        &["apply", "--dry-run=client", "-R", "-f", &path],
        None,
    )
    .unwrap_or_else(|error| panic!("kubectl rejects the fixture: {error}"));
    assert_eq!(
        out.lines().filter(|l| l.contains("(dry run)")).count(),
        fixture::MANIFEST_COUNT - 5,
        "{out}"
    );
}

/// A manifest as `kubectl` would take it: the server's fields removed.
fn without_server_fields(mut object: Value) -> Value {
    if let Some(meta) = object["metadata"].as_object_mut() {
        for key in [
            "uid",
            "resourceVersion",
            "creationTimestamp",
            "managedFields",
            "generation",
            "selfLink",
        ] {
            meta.remove(key);
        }
    }
    object.as_object_mut().map(|o| o.remove("status"));
    object
}

fn assert_kubectl_accepts(context: &str, namespace: &str, copied: &Value) {
    assert!(
        copied["metadata"].get("uid").is_some(),
        "read back from the cluster"
    );
    let manifest = without_server_fields(copied.clone());
    kubectl(
        context,
        &["-n", namespace, "apply", "--dry-run=server", "-f", "-"],
        Some(&manifest.to_string()),
    )
    .unwrap_or_else(|error| panic!("kubectl does not accept the copy: {error}"));
}

#[tokio::test]
async fn a_deployment_and_a_configmap_are_copied_to_the_other_namespace() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let context = kind.context.to_string();
    let run = random_namespace_name();
    let _cleanup = Cleanup {
        context: context.clone(),
        run: run.clone(),
    };
    let log = Log::default();
    let _diagnostics = Diagnostics {
        run: run.clone(),
        log: log.clone(),
    };
    let [app, data] = fixture::namespaces(&run);
    let apply = ApplyRig::new(&kind).await;
    let report = apply_docs(
        &apply,
        &log,
        "fixture",
        fixture::docs(&run),
        &ApplyOptions::default(),
    )
    .await;
    assert_eq!(report.failures(), 0, "{}", words(&report));

    let copy = CopyRig::new(&kind).await;
    let gvk = |api: &str, kind: &str| Gvk::from_api_version(api, kind);
    for (gvk, name) in [
        (gvk("v1", "ConfigMap"), "settings-1"),
        (gvk("apps/v1", "Deployment"), "web-1"),
    ] {
        let preview = copy
            .service
            .preview(copy.request(gvk, &app, &data, name))
            .await
            .expect("preview");
        let outcome = copy
            .service
            .copy(&preview, None, ReviewAnswer::click())
            .await
            .expect("copy");
        assert!(
            matches!(outcome, CopyOutcome::Copied { created: true, .. }),
            "{outcome:?}"
        );
    }
    let source = get_json(&context, &app, "configmap/settings-1");
    let copied = get_json(&context, &data, "configmap/settings-1");
    assert_eq!(copied["data"], source["data"]);
    assert_ne!(copied["metadata"]["uid"], source["metadata"]["uid"]);
    assert_eq!(copied["metadata"]["labels"], source["metadata"]["labels"]);
    assert_kubectl_accepts(&context, &data, &copied);
    let deployment = get_json(&context, &data, "deployment/web-1");
    assert_eq!(
        deployment["spec"]["selector"],
        json!({"matchLabels": {"app": "web-1"}})
    );
    assert_kubectl_accepts(&context, &data, &deployment);
}

#[tokio::test]
async fn copy_to_a_second_cluster() {
    let Some(context_b) = std::env::var("OXIKUBE_TEST_CONTEXT_B")
        .ok()
        .filter(|c| !c.trim().is_empty())
    else {
        eprintln!("skipping the cross-cluster copy: OXIKUBE_TEST_CONTEXT_B is not set");
        return;
    };
    let Some(kind_a) = Kind::from_env().await else {
        return;
    };
    let kind_b = Kind::named(context_b.clone()).await;
    assert_ne!(kind_a.context, kind_b.context, "two different clusters");
    let context_a = kind_a.context.to_string();
    let run = random_namespace_name();
    let _cleanup_a = Cleanup {
        context: context_a.clone(),
        run: run.clone(),
    };
    let _cleanup_b = Cleanup {
        context: context_b.clone(),
        run: run.clone(),
    };
    let log = Log::default();
    let _diagnostics = Diagnostics {
        run: run.clone(),
        log: log.clone(),
    };
    let [app, data] = fixture::namespaces(&run);
    let apply = ApplyRig::new(&kind_a).await;
    let report = apply_docs(
        &apply,
        &log,
        "fixture",
        fixture::docs(&run),
        &ApplyOptions::default(),
    )
    .await;
    assert_eq!(report.failures(), 0, "{}", words(&report));
    kubectl(&context_b, &["create", "namespace", &data], None).expect("namespace on B");

    let copy = CopyRig::across(&kind_a, &kind_b).await;
    for (api, kind, name) in [
        ("v1", "ConfigMap", "settings-1"),
        ("apps/v1", "Deployment", "web-1"),
    ] {
        let source = ResourceRef::namespaced(
            copy.cluster.clone(),
            Gvk::from_api_version(api, kind),
            app.as_str(),
            name,
        );
        let request = oxikube_app::copy::CopyRequest::to(source, copy.other.clone(), Some(&data));
        let preview = copy.service.preview(request).await.expect("preview");
        copy.service
            .copy(&preview, None, ReviewAnswer::click())
            .await
            .expect("copy to B");
    }
    for what in ["configmap/settings-1", "deployment/web-1"] {
        let copied = get_json(&context_b, &data, what);
        assert_kubectl_accepts(&context_b, &data, &copied);
    }
    // A is untouched: the copy went to B only.
    assert!(
        kubectl(&context_a, &["-n", &data, "get", "deployment/web-1"], None).is_err(),
        "the Deployment is not in A's {data}"
    );
}
