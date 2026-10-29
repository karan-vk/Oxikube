//! The audit log viewer, rendered through `Window::render_to_image` (E19-S09): twelve records with
//! mixed outcomes (succeeded, failed, denied by read-only mode, cancelled), doors and verbs, one
//! of a batch, the second row selected so the detail panel shows.
//!
//! - `audit_dark`, `audit_light`: the viewer in each appearance.
//! - `audit_unavailable`: the "database unavailable" state, dark.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests (see
//! `oxikube_catalog_ui/tests/catalog_screenshot.rs` for why). Checks the image size, that it is not
//! blank and, when a golden exists for this OS, that it matches within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use jiff::{SignedDuration, Timestamp};
use oxikube_app::audit::query::AuditReader;
use oxikube_app::{AuditLog, ClusterSessionManager};
use oxikube_domain::OxiError;
use oxikube_domain::audit::{AuditDenial, AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::safety::MutationVerb;
use oxikube_ports::{ClusterContext, SourceId, StatePort as _};
use oxikube_resources_ui::audit::{AuditView, AuditViewDeps};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort, FakeStatePort,
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 640.0;

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn now() -> Timestamp {
    "2026-10-30T12:00:00Z"
        .parse()
        .expect("the fixed clock is an RFC 3339 timestamp")
}

/// `(command, namespace, kind, name, initiator, outcome, denial, dry run, who, batch, minutes ago)`.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Initiator,
    AuditOutcome,
    Option<AuditDenial>,
    bool,
    &'static str,
    Option<&'static str>,
    i64,
);

const ROWS: [Row; 12] = [
    (
        "pod::Delete",
        "shop",
        "Pod",
        "cart-6d9f-x2k4q",
        Initiator::Ui,
        AuditOutcome::Succeeded,
        None,
        false,
        "alice",
        None,
        1,
    ),
    (
        "workload::Scale",
        "shop",
        "Deployment",
        "cart",
        Initiator::Command,
        AuditOutcome::Succeeded,
        None,
        false,
        "alice",
        None,
        3,
    ),
    (
        "pod::Delete",
        "batch",
        "Pod",
        "etl-28843-zz8kq",
        Initiator::Agent,
        AuditOutcome::Denied,
        Some(AuditDenial::ReadOnly),
        false,
        "claude",
        None,
        4,
    ),
    (
        "selection::Delete",
        "web",
        "Pod",
        "web-5c8b7-q9zt2",
        Initiator::Ui,
        AuditOutcome::Succeeded,
        None,
        false,
        "alice",
        Some("batch-7f3a"),
        6,
    ),
    (
        "selection::Delete",
        "web",
        "Pod",
        "web-5c8b7-r2p9m",
        Initiator::Ui,
        AuditOutcome::Succeeded,
        None,
        false,
        "alice",
        Some("batch-7f3a"),
        6,
    ),
    (
        "selection::Delete",
        "web",
        "Pod",
        "web-5c8b7-t4n8c",
        Initiator::Ui,
        AuditOutcome::Failed,
        None,
        false,
        "alice",
        Some("batch-7f3a"),
        6,
    ),
    (
        "node::Drain",
        "",
        "Node",
        "worker-3",
        Initiator::Ui,
        AuditOutcome::Cancelled,
        Some(AuditDenial::UserCancelled),
        false,
        "alice",
        None,
        14,
    ),
    (
        "resource::Apply",
        "shop",
        "ConfigMap",
        "cart-config",
        Initiator::Ui,
        AuditOutcome::Succeeded,
        None,
        true,
        "alice",
        None,
        22,
    ),
    (
        "workload::Restart",
        "web",
        "Deployment",
        "web",
        Initiator::Plugin,
        AuditOutcome::Denied,
        Some(AuditDenial::Policy),
        false,
        "ext.restarter",
        None,
        35,
    ),
    (
        "workload::Scale",
        "shop",
        "Deployment",
        "checkout",
        Initiator::Agent,
        AuditOutcome::Failed,
        None,
        false,
        "claude",
        None,
        41,
    ),
    (
        "helm::Rollback",
        "shop",
        "Release",
        "shop-stack",
        Initiator::Command,
        AuditOutcome::Succeeded,
        None,
        false,
        "alice",
        None,
        58,
    ),
    (
        "pod::Evict",
        "kube-system",
        "Pod",
        "coredns-5dd5-8xk2z",
        Initiator::Ui,
        AuditOutcome::Succeeded,
        None,
        false,
        "alice",
        None,
        90,
    ),
];

fn records(cluster: &ClusterId) -> Vec<AuditRecord> {
    ROWS.iter()
        .rev()
        .map(
            |(cmd, ns, kind, name, initiator, outcome, denial, dry, who, batch, ago)| {
                let gvk = Gvk::from_api_version("v1", kind);
                let target = if ns.is_empty() {
                    ResourceRef::cluster_scoped(cluster.clone(), gvk, *name)
                } else {
                    ResourceRef::namespaced(cluster.clone(), gvk, *ns, *name)
                };
                let mut record = AuditRecord::new(
                    now() - SignedDuration::from_mins(*ago),
                    who,
                    *initiator,
                    cmd,
                    target,
                    *dry,
                    *outcome,
                );
                if let Some(verb) = MutationVerb::for_command(cmd) {
                    record = record.with_verb(verb);
                }
                if let Some(denial) = denial {
                    record = record.with_denial(*denial);
                }
                if let Some(batch) = batch {
                    record = record.with_batch_id(batch);
                }
                record
            },
        )
        .collect()
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn render(tokens: oxikube_ui::Tokens, unavailable: bool) -> anyhow::Result<RgbaImage> {
    let cluster = ClusterId::new("/shots/kubeconfig", &ContextName::new("shots"));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([ClusterContext::new(
            cluster.clone(),
            ContextName::new("shots"),
            SourceId("k".into()),
        )]),
    );
    let clock = Arc::new(FakeClockPort::new(now()));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    futures::executor::block_on(sessions.connect(&cluster))?;
    let state = Arc::new(FakeStatePort::new());
    futures::executor::block_on(state.append_audit(&records(&cluster)))?;
    if unavailable {
        state
            .script()
            .query_audit
            .push_err(OxiError::internal("unable to open database file"));
    }
    let log = AuditLog::new(state.clone(), clock.clone());
    let deps = AuditViewDeps {
        reader: AuditReader::new(state),
        listener: log.appended(),
        sessions,
        dispatcher: Rc::new(Ignore),
        fs: Arc::new(FakeFsPort::new()),
        clock,
    };

    let mut cx = headless();
    let mut view = None;
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let audit = cx.new(|cx| AuditView::new(deps, None, window, cx));
        view = Some(audit.clone());
        oxikube_ui::root::new_root(audit, window, cx)
    })?;
    cx.run_until_parked();
    let view = view.expect("the view");
    if !unavailable {
        view.update(&mut cx, |v, cx| v.select(Some(2), cx));
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
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

fn run() -> anyhow::Result<()> {
    check("audit_dark", render(oxikube_ui::Tokens::dark(), false)?)?;
    check("audit_light", render(oxikube_ui::Tokens::light(), false)?)?;
    check(
        "audit_unavailable",
        render(oxikube_ui::Tokens::dark(), true)?,
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("audit screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("audit screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
