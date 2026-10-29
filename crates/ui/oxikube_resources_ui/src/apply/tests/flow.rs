//! The panel from the file to the result: load a directory, review the dry run, confirm, apply,
//! read the rows, expand a failure, filter, copy, re-run, cancel.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::TestAppContext;
use oxikube_app::apply::{ApplyOptions, ClusterPlan, PlanTarget};
use oxikube_domain::OxiError;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::kustomize::KustomizeTool;
use oxikube_domain::manifests::ManifestDoc;
use oxikube_ports::CancellationToken;
use parking_lot::Mutex;

use super::{CM_A, CM_B, NS, Rig, seed};
use crate::apply::{
    Activity, ApplyBackend, ApplySource, Loaded, ResultStream, ServiceBackend, SourceKind, Stage,
};
use crate::table::tests::fixture::cluster;

fn two_maps(rig: &Rig) {
    seed(rig, "/m", &[("a.yaml", CM_A), ("b.yaml", CM_B)]);
}

#[gpui::test]
fn a_directory_loads_reviews_applies_and_shows_every_object(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    seed(
        &rig,
        "/m",
        &[("a.yaml", CM_A), ("b.yaml", CM_B), ("ns.yaml", NS)],
    );
    rig.open("/m");

    // Review: the plan in kind order (the Namespace first), each dry-run answer in its row.
    assert!(rig.is_laid_out("apply-panel"));
    assert_eq!(rig.read(|v| v.model().stage()), Stage::Review);
    assert_eq!(rig.words(), ["created", "created", "created"]);
    let names: Vec<_> = rig.read(|v| v.model().rows().iter().map(|r| r.label()).collect());
    assert_eq!(
        names,
        [
            "Namespace shop",
            "ConfigMap default/a",
            "ConfigMap default/b"
        ]
    );
    assert!(rig.is_laid_out("apply-row-0"));
    assert!(rig.is_laid_out("apply-status-0-created"));
    assert!(
        rig.is_laid_out("apply-cluster-scoped-0"),
        "the Namespace says it is cluster-scoped"
    );
    assert!(!rig.is_laid_out("apply-cluster-scoped-1"));
    assert!(
        rig.writes().iter().all(|dry| *dry),
        "the review wrote nothing: {:?}",
        rig.writes()
    );
    assert!(!rig.writes().is_empty(), "the server was asked to dry-run");

    // Apply asks first, showing what the dry run said.
    rig.click("apply-apply");
    assert!(rig.is_laid_out("apply-confirmation"));
    assert!(
        rig.writes().iter().all(|dry| *dry),
        "nothing is written before the confirmation"
    );
    rig.click("apply-decline");
    assert!(!rig.is_laid_out("apply-confirmation"));
    assert!(rig.writes().iter().all(|dry| *dry));

    rig.click("apply-apply");
    rig.click("apply-confirm");
    assert_eq!(rig.read(|v| v.model().stage()), Stage::Result);
    assert_eq!(rig.read(|v| v.model().activity()), Activity::Idle);
    assert_eq!(rig.words(), ["created", "created", "created"]);
    assert!(
        rig.writes().iter().any(|dry| !*dry),
        "the apply wrote for real"
    );
    assert!(rig.read(|v| v.model().rows().iter().all(|r| r.elapsed.is_some())));
    let summary = rig.read(|v| v.model().summary_line());
    assert_eq!(summary, "3 created");
}

#[gpui::test]
fn parse_errors_are_listed_with_file_and_line_and_the_rest_still_applies(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    seed(
        &rig,
        "/m",
        &[
            ("a.yaml", CM_A),
            ("broken.yaml", "kind: ConfigMap\nmetadata: [unclosed\n"),
        ],
    );
    rig.open("/m");
    assert_eq!(rig.words(), ["created"], "the readable object is planned");
    rig.click("apply-stage-load");
    assert_eq!(rig.read(|v| v.model().stage()), Stage::Load);
    assert!(rig.is_laid_out("apply-load-summary"));
    assert!(rig.is_laid_out("apply-load-error-0"));
    let error = rig.read(|v| v.model().summary().unwrap().errors[0].to_string());
    assert!(error.contains("/m/broken.yaml:"), "{error}");
    let (files, documents) = rig.read(|v| {
        let s = v.model().summary().unwrap();
        (s.files, s.documents)
    });
    assert_eq!((files, documents), (2, 1));
}

#[gpui::test]
fn a_missing_path_says_so_in_the_load_view(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    rig.open("/nowhere");
    assert!(rig.is_laid_out("apply-problem"));
    assert_eq!(rig.read(|v| v.model().stage()), Stage::Load);
    assert!(rig.read(|v| v.apply_blocked()).is_some());
}

fn script_bad_first(rig: &Rig, times: usize) {
    for _ in 0..times {
        rig.f
            .ports()
            .resources
            .script()
            .patch
            .push_err(OxiError::validation(
                "ConfigMap \"a\" is invalid: data.k: Invalid value",
            ));
    }
}

#[gpui::test]
fn a_failed_object_expands_to_its_message_and_the_errors_only_filter_hides_the_rest(
    cx: &mut TestAppContext,
) {
    let mut rig = Rig::new(cx, None);
    two_maps(&rig);
    script_bad_first(&rig, 1);
    rig.open("/m");
    assert_eq!(
        rig.words(),
        ["failed", "created"],
        "the dry run failed the first object"
    );
    assert!(rig.is_laid_out("apply-status-0-failed"));

    // Applying: the first object fails again, the second goes on (as kubectl apply).
    script_bad_first(&rig, 1);
    rig.click("apply-apply");
    rig.click("apply-confirm");
    assert_eq!(rig.words(), ["failed", "created"]);
    assert!(!rig.is_laid_out("apply-detail-0-1"), "closed until opened");
    rig.click("apply-row-0");
    assert!(
        rig.is_laid_out("apply-detail-0-1"),
        "the full message is a line under the row"
    );
    rig.click("apply-row-0");
    assert!(!rig.is_laid_out("apply-detail-0-1"));

    rig.click("apply-errors-only");
    assert!(rig.is_laid_out("apply-row-0"));
    assert!(
        !rig.is_laid_out("apply-row-1"),
        "the created object is filtered out"
    );
    rig.click("apply-errors-only");
    assert!(rig.is_laid_out("apply-row-1"));

    rig.click("apply-copy");
    let text = rig
        .f
        .vcx
        .read(|cx| cx.read_from_clipboard())
        .and_then(|c| c.text())
        .unwrap_or_default();
    assert!(text.contains("failed\tConfigMap default/a"), "{text}");
    assert!(text.contains("is invalid"), "{text}");
    assert!(text.contains("created\tConfigMap default/b"), "{text}");
}

#[gpui::test]
fn rerun_reads_the_files_again_and_rerun_failed_keeps_only_what_failed(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    two_maps(&rig);
    script_bad_first(&rig, 1);
    rig.open("/m");
    script_bad_first(&rig, 1);
    rig.click("apply-apply");
    rig.click("apply-confirm");
    assert_eq!(rig.words(), ["failed", "created"]);

    // The user fixes a file and adds another: Re-run plans everything from disk again.
    rig.file(
        "/m/c.yaml",
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: c\n",
    );
    rig.click("apply-rerun");
    assert_eq!(
        rig.read(|v| v.model().stage()),
        Stage::Review,
        "re-run rehearses; Apply stays the user's"
    );
    assert_eq!(
        rig.words(),
        ["created", "unchanged", "created"],
        "b was applied the first time"
    );

    // After another failed apply, only the failed object is planned.
    script_bad_first(&rig, 1);
    rig.click("apply-apply");
    rig.click("apply-confirm");
    assert_eq!(rig.words(), ["failed", "unchanged", "created"]);
    rig.click("apply-rerun-failed");
    let names: Vec<_> = rig.read(|v| v.model().rows().iter().map(|r| r.name.clone()).collect());
    assert_eq!(names, ["a"]);
}

#[gpui::test]
fn the_namespace_field_overrides_where_namespaced_objects_go(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    two_maps(&rig);
    rig.open("/m");
    assert_eq!(
        rig.read(|v| v.model().rows()[0].namespace.clone())
            .as_deref(),
        Some("default")
    );
    let input = rig.read(|v| v.namespace.clone());
    rig.f.vcx.update(|window, cx| {
        input.update(cx, |input, cx| input.set_value("prod", window, cx));
    });
    rig.update(|view, cx| view.replan(cx));
    let namespaces: Vec<_> = rig.read(|v| {
        v.model()
            .rows()
            .iter()
            .map(|r| r.namespace.clone())
            .collect()
    });
    assert_eq!(
        namespaces,
        [Some("prod".to_owned()), Some("prod".to_owned())]
    );
    assert_eq!(
        rig.read(|v| v.plan().unwrap().target.namespace.clone())
            .as_deref(),
        Some("prod")
    );
}

#[gpui::test]
fn the_field_starts_at_the_tabs_selected_namespace(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    rig.f
        .sessions
        .set_namespace_selection(
            &cluster(),
            oxikube_domain::session::NamespaceSelection::single("shop"),
        )
        .expect("selected");
    // A panel opened now starts at the selected namespace.
    rig.request(crate::apply::SourceKind::Manifests, Some("/nowhere"));
    let input = rig.read(|v| v.namespace.clone());
    let text = rig.f.vcx.read(|cx| input.read(cx).value().to_string());
    assert_eq!(text, "shop");
}

#[gpui::test]
fn the_directory_is_remembered_per_cluster_in_the_state_port(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    two_maps(&rig);
    rig.open("/m");
    let key = oxikube_ports::StateKey::new(format!("apply.last_dir.{}", cluster().as_str()))
        .expect("a valid key");
    let stored = futures::executor::block_on(oxikube_ports::StatePort::kv_get(&*rig.state, &key))
        .expect("read");
    assert_eq!(stored, Some(serde_json::json!("/m")));

    // A file remembers the directory it is in.
    rig.open("/m/a.yaml");
    let stored = futures::executor::block_on(oxikube_ports::StatePort::kv_get(&*rig.state, &key))
        .expect("read");
    assert_eq!(stored, Some(serde_json::json!("/m")));
}

// A backend that holds the dry run (and the apply) until the test lets it go.
struct Held {
    inner: ServiceBackend,
    release: Mutex<Option<oneshot::Receiver<()>>>,
    seen: Arc<Mutex<Vec<CancellationToken>>>,
    hold_apply: bool,
}

impl Held {
    /// The gate, for the dry run (`apply` false) or the apply (true) the test holds.
    fn take_gate(&self, apply: bool) -> Option<oneshot::Receiver<()>> {
        if self.hold_apply == apply {
            self.release.lock().take()
        } else {
            None
        }
    }
}

impl ApplyBackend for Held {
    fn load(&self, source: ApplySource) -> BoxFuture<'static, oxikube_domain::OxiResult<Loaded>> {
        self.inner.load(source)
    }
    fn plan(&self, docs: Vec<ManifestDoc>, target: PlanTarget) -> BoxFuture<'static, ClusterPlan> {
        self.inner.plan(docs, target)
    }
    fn dry_run(&self, plan: ClusterPlan, options: ApplyOptions) -> ResultStream {
        self.seen.lock().push(options.cancel.clone());
        let inner = self.inner.dry_run(plan, options);
        match self.take_gate(false) {
            Some(gate) => futures::stream::once(async move {
                let _ = gate.await;
            })
            .flat_map(move |()| futures::stream::empty())
            .chain(inner)
            .boxed(),
            None => inner,
        }
    }
    fn apply(&self, plan: ClusterPlan, options: ApplyOptions) -> ResultStream {
        self.seen.lock().push(options.cancel.clone());
        let inner = self.inner.apply(plan, options);
        match self.take_gate(true) {
            Some(gate) => futures::stream::once(async move {
                let _ = gate.await;
            })
            .flat_map(move |()| futures::stream::empty())
            .chain(inner)
            .boxed(),
            None => inner,
        }
    }
    fn kustomize_tool(&self) -> BoxFuture<'static, Option<KustomizeTool>> {
        self.inner.kustomize_tool()
    }
    fn read_text(
        &self,
        path: std::path::PathBuf,
    ) -> BoxFuture<'static, oxikube_domain::OxiResult<String>> {
        self.inner.read_text(path)
    }
    fn last_directory(&self, cluster: ClusterId) -> BoxFuture<'static, Option<std::path::PathBuf>> {
        self.inner.last_directory(cluster)
    }
    fn remember_directory(
        &self,
        cluster: ClusterId,
        path: std::path::PathBuf,
    ) -> BoxFuture<'static, ()> {
        self.inner.remember_directory(cluster, path)
    }
    fn read_only(&self, cluster: &ClusterId) -> bool {
        self.inner.read_only(cluster)
    }
    fn cluster_label(&self, cluster: &ClusterId) -> String {
        self.inner.cluster_label(cluster)
    }
    fn selected_namespace(&self, cluster: &ClusterId) -> Option<String> {
        self.inner.selected_namespace(cluster)
    }
}

fn held(
    cx: &mut TestAppContext,
    hold_apply: bool,
) -> (Rig, oneshot::Sender<()>, Arc<Mutex<Vec<CancellationToken>>>) {
    let (open, gate) = oneshot::channel();
    let seen: Arc<Mutex<Vec<CancellationToken>>> = Arc::default();
    let tokens = seen.clone();
    let rig = Rig::wrapped(cx, None, move |inner| {
        std::rc::Rc::new(Held {
            inner,
            release: Mutex::new(Some(gate)),
            seen: tokens,
            hold_apply,
        })
    });
    (rig, open, seen)
}

#[gpui::test]
fn cancel_stops_a_dry_run_at_once_and_marks_what_is_left_skipped(cx: &mut TestAppContext) {
    let (mut rig, release, _) = held(cx, false);
    two_maps(&rig);
    rig.open("/m");
    assert_eq!(rig.read(|v| v.model().activity()), Activity::DryRunning);
    assert!(rig.is_laid_out("apply-cancel"));
    assert_eq!(rig.words(), ["waiting", "waiting"]);
    rig.click("apply-cancel");
    assert_eq!(rig.read(|v| v.model().activity()), Activity::Idle);
    assert_eq!(rig.words(), ["skipped", "skipped"]);
    drop(release);
    rig.settle();
    assert_eq!(
        rig.words(),
        ["skipped", "skipped"],
        "nothing arrives after the cancel"
    );
}

#[gpui::test]
fn cancel_during_the_apply_cancels_the_runs_token_between_objects(cx: &mut TestAppContext) {
    let (mut rig, release, seen) = held(cx, true);
    two_maps(&rig);
    rig.open("/m");
    assert_eq!(
        rig.read(|v| v.model().activity()),
        Activity::Idle,
        "the dry run was not held"
    );
    rig.click("apply-apply");
    rig.click("apply-confirm");
    assert_eq!(rig.read(|v| v.model().activity()), Activity::Applying);
    rig.click("apply-cancel");
    let apply_token = seen.lock().last().cloned().expect("the apply was started");
    assert!(
        apply_token.is_cancelled(),
        "the service stops between objects"
    );
    let _ = release.send(());
    rig.settle();
}

#[gpui::test]
fn loading_another_source_is_refused_while_an_apply_runs(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    two_maps(&rig);
    rig.open("/m");
    let rows = rig.read(|v| v.model().rows().len());
    rig.update(|view, _| view.model_mut().set_activity(Activity::Applying));
    rig.request(SourceKind::Manifests, Some("/m"));
    rig.settle();
    assert_eq!(rig.read(|v| v.model().activity()), Activity::Applying);
    assert_eq!(rig.read(|v| v.model().rows().len()), rows, "results kept");
    assert!(rig.read(|v| v.problem.is_some()));
}

#[gpui::test]
fn rerun_failed_tells_the_same_name_in_two_namespaces_apart(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx, None);
    let map = |namespace: &str| {
        format!(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: app\n  namespace: {namespace}\n"
        )
    };
    seed(
        &rig,
        "/m",
        &[("a.yaml", &map("dev")), ("b.yaml", &map("prod"))],
    );
    rig.open("/m");
    script_bad_first(&rig, 1);
    rig.click("apply-apply");
    rig.click("apply-confirm");
    assert_eq!(rig.words(), ["failed", "created"]);
    rig.click("apply-rerun-failed");
    let spaces: Vec<_> = rig.read(|v| {
        v.model()
            .rows()
            .iter()
            .map(|row| row.namespace.clone())
            .collect()
    });
    assert_eq!(spaces, [Some("dev".to_owned())], "only the failed one");
}
