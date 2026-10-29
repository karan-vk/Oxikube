//! The apply panel (E20-S05), rendered through `Window::render_to_image` over a scripted backend.
//!
//! - `apply_review`: the plan in kind order with the server's dry run in each row (created,
//!   configured, unchanged, deferred), the Namespace marked cluster-scoped, Apply ready.
//! - `apply_result`: after the apply, mixed statuses with a failure and a skipped object.
//! - `apply_error_expanded`: the failed row open under its message and field paths.
//! - `apply_read_only`: the lock, the plan without a dry run and Apply disabled with the reason.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests. Checks the
//! image size, that it is not blank and, when a golden exists for this OS, that it matches within
//! tolerance. Regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a harness-less test binary reports its results on stdout and stderr"
)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::apply::{ApplyOptions, ClusterPlan, ObjectResult, ObjectStatus, PlanTarget};
use oxikube_domain::OxiResult;
use oxikube_domain::apply::ApplyPlan;
use oxikube_domain::ids::{ClusterId, ContextName, Scope};
use oxikube_domain::kustomize::KustomizeTool;
use oxikube_domain::manifests::{ManifestDoc, SourceId};
use oxikube_resources_ui::apply::{
    ApplyBackend, ApplyDeps, ApplySource, ApplyView, Loaded, ResultStream,
};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use serde_json::{Value, json};

const WIDTH: f32 = 1180.0;
const HEIGHT: f32 = 560.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scene {
    Review,
    Result,
    ErrorExpanded,
    ReadOnly,
}

fn object(kind: &str, name: &str, namespace: Option<&str>) -> Value {
    let mut object = json!({"apiVersion": "v1", "kind": kind, "metadata": {"name": name}});
    if let Some(namespace) = namespace {
        object["metadata"]["namespace"] = json!(namespace);
    }
    object
}

fn docs() -> Vec<ManifestDoc> {
    let at = |file: &str, line: usize, json: Value| ManifestDoc {
        source: SourceId::new(format!("/work/shop/{file}")),
        line,
        json,
    };
    vec![
        at("web.yaml", 1, object("ConfigMap", "web-config", None)),
        at("namespace.yaml", 1, object("Namespace", "shop", None)),
        at("web.yaml", 9, object("Service", "web", None)),
        at("web.yaml", 21, object("ConfigMap", "feature-flags", None)),
        at("jobs.yaml", 1, object("ConfigMap", "migrate-sql", None)),
        at("db.yaml", 1, object("Secret", "db-credentials", None)),
    ]
}

fn result(index: usize, dry_run: bool, status: ObjectStatus) -> ObjectResult {
    ObjectResult {
        index,
        label: format!("object {index}"),
        target: None,
        dry_run,
        status,
        method: None,
    }
}

fn failed() -> ObjectStatus {
    ObjectStatus::Failed {
        reason: "ConfigMap \"web-config\" is invalid: data[bad key]: Invalid value: \"bad key\": a valid config key must consist of alphanumeric characters, '-', '_' or '.'".to_owned(),
        fields: vec!["data[bad key]".to_owned()],
        conflicts: Vec::new(),
    }
}

/// A backend that answers from a script: one directory, the dry run and the apply as listed.
struct Scripted {
    read_only: bool,
    dry: Vec<ObjectStatus>,
    done: Vec<ObjectStatus>,
}

impl ApplyBackend for Scripted {
    fn load(&self, _: ApplySource) -> BoxFuture<'static, OxiResult<Loaded>> {
        let loaded = Loaded {
            docs: docs(),
            files: 4,
            ..Loaded::default()
        };
        async move { Ok(loaded) }.boxed()
    }
    fn plan(&self, docs: Vec<ManifestDoc>, target: PlanTarget) -> BoxFuture<'static, ClusterPlan> {
        let namespace = target.namespace.clone();
        async move {
            let plan = ApplyPlan::build(docs, namespace.as_deref().or(Some("shop")), |gvk| {
                Some(if &*gvk.kind == "Namespace" {
                    Scope::Cluster
                } else {
                    Scope::Namespaced
                })
            });
            ClusterPlan { target, plan }
        }
        .boxed()
    }
    fn dry_run(&self, _: ClusterPlan, _: ApplyOptions) -> ResultStream {
        let results: Vec<_> = self
            .dry
            .iter()
            .enumerate()
            .map(|(index, status)| Ok(result(index, true, status.clone())))
            .collect();
        futures::stream::iter(results).boxed()
    }
    fn apply(&self, _: ClusterPlan, _: ApplyOptions) -> ResultStream {
        let results: Vec<_> = self
            .done
            .iter()
            .enumerate()
            .map(|(index, status)| Ok(result(index, false, status.clone())))
            .collect();
        futures::stream::iter(results).boxed()
    }
    fn kustomize_tool(&self) -> BoxFuture<'static, Option<KustomizeTool>> {
        async { None }.boxed()
    }
    fn read_text(&self, _: PathBuf) -> BoxFuture<'static, OxiResult<String>> {
        async { Ok(String::new()) }.boxed()
    }
    fn last_directory(&self, _: ClusterId) -> BoxFuture<'static, Option<PathBuf>> {
        async { None }.boxed()
    }
    fn remember_directory(&self, _: ClusterId, _: PathBuf) -> BoxFuture<'static, ()> {
        async {}.boxed()
    }
    fn read_only(&self, _: &ClusterId) -> bool {
        self.read_only
    }
    fn cluster_label(&self, _: &ClusterId) -> String {
        "kind-oxikube".to_owned()
    }
    fn selected_namespace(&self, _: &ClusterId) -> Option<String> {
        Some("shop".to_owned())
    }
}

fn backend(scene: Scene) -> Scripted {
    use ObjectStatus::{Configured, Created, DeferredDryRun, Skipped, Unchanged};
    // Plan order: Namespace, then the namespaced kinds in kind order.
    let dry = vec![
        Created,
        Created,
        Configured,
        Unchanged,
        DeferredDryRun,
        failed(),
    ];
    let done = vec![
        Created,
        Created,
        Configured,
        Unchanged,
        failed(),
        Skipped {
            reason: "not applied: ConfigMap shop/migrate-sql failed".to_owned(),
        },
    ];
    Scripted {
        read_only: scene == Scene::ReadOnly,
        dry,
        done,
    }
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(scene: Scene) -> anyhow::Result<RgbaImage> {
    let cluster = ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"));
    let held: Rc<RefCell<Option<gpui::Entity<ApplyView>>>> = Rc::default();
    let hold = held.clone();
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = ApplyDeps {
            backend: Rc::new(backend(scene)),
            cluster,
            editor: Rc::new(|_, _| {}),
        };
        let view = cx.new(|cx| ApplyView::new(deps, window, cx));
        *hold.borrow_mut() = Some(view.clone());
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    let view = held.borrow().clone().expect("the apply view");
    cx.update_window(window.into(), |_, _, cx| {
        view.update(cx, |view, cx| {
            view.load(ApplySource::manifests("/work/shop"), cx);
        });
    })?;
    cx.run_until_parked();
    if matches!(scene, Scene::Result | Scene::ErrorExpanded) {
        cx.update_window(window.into(), |_, _, cx| {
            view.update(cx, |view, cx| {
                view.request_apply(cx);
                view.confirm_apply(cx);
            });
        })?;
        cx.run_until_parked();
    }
    if scene == Scene::ErrorExpanded {
        cx.update_window(window.into(), |_, _, cx| {
            view.update(cx, |view, cx| {
                view.select(4, cx);
                view.toggle_expanded(4, cx);
            });
        })?;
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    let shot = cx.capture_screenshot(window.into());
    // The test's handle must not outlive the app, or GPUI reports a leaked entity.
    held.borrow_mut().take();
    shot
}

fn check(name: &str, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(
        distinct_colors_at_least(&image, 8),
        "{name}: frame looks blank"
    );
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, name);
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, Tolerance::default())?;
        println!("{name} matches {}", golden.display());
    } else {
        println!(
            "no golden for {} yet; structural checks only",
            std::env::consts::OS
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    for (name, scene) in [
        ("apply_review", Scene::Review),
        ("apply_result", Scene::Result),
        ("apply_error_expanded", Scene::ErrorExpanded),
        ("apply_read_only", Scene::ReadOnly),
    ] {
        if let Err(err) = render(scene).and_then(|image| check(name, image)) {
            eprintln!("{name} screenshot failed: {err:#}");
            return ExitCode::FAILURE;
        }
    }
    println!("apply screenshots: ok");
    ExitCode::SUCCESS
}
