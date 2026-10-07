//! Screenshots of the log view (E08-S02), rendered through `Window::render_to_image`.
//!
//! - `log_view_levels`: a pod's log with info, warning, error and debug lines and one long line,
//!   unwrapped, timestamps shown, the "stream ended" state row at the bottom (dark theme).
//! - `log_view_wrapped`: the same log wrapped, timestamps hidden (dark theme).
//! - `log_view_light`: the same log unwrapped in the light theme.
//! - `log_view_search`: the search bar open with `error|warn` (E08-S03): the matches highlighted,
//!   the current match's row tinted, the count in the bar (dark theme).
//! - `log_view_filter`: the same search in filter mode: only the matching lines are rows.
//! - `log_view_json` (E08-S05): a mixed stream of zap, logrus and pino JSON lines and plain text in
//!   JSON mode: level chips, time and message columns, the level chips with `debug` off, and one
//!   line expanded into its pretty-printed JSON.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device (Metal, or Vulkan such as Mesa lavapipe on Linux), so it only builds
//! with `--features screenshot` and runs in the nightly job:
//! `cargo test -p oxikube_logs_ui --features screenshot --test screenshot`.
//!
//! Regenerate the goldens with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, HeadlessAppContext, px, size};
use jiff::Timestamp;
use oxikube_app::ClusterSessionManager;
use oxikube_app::logs::{LogConfig, LogService};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::log::{LevelChip, LogLine};
use oxikube_logs_ui::{LogView, LogViewDeps, SearchMode, log_runtime};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, Timeline};
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 360.0;
/// The JSON scene is taller: the expanded line's pane takes 240 px under the rows.
const JSON_HEIGHT: f32 = 580.0;

struct Ignore;

impl CommandDispatcher for Ignore {
    fn dispatch(&self, _: Command, _: &mut App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

/// A mixed stream: a plain banner, then JSON lines in the shapes of zap, logrus and pino.
fn json_log() -> Vec<LogLine> {
    let texts = [
        "=== orders-api starting (build 3f9c2ab) ===",
        r#"{"level":"info","ts":1791105243.12,"caller":"server/main.go:42","msg":"starting server","addr":":8080","version":"1.42.0"}"#,
        r#"{"level":"debug","ts":1791105243.4,"caller":"cache/lru.go:31","msg":"cache warmed","entries":12480}"#,
        r#"{"level":"info","msg":"connected to postgres","time":"2026-10-07T09:14:04.5Z","host":"orders-db","port":5432}"#,
        r#"{"level":"warning","msg":"slow query on orders_by_customer","time":"2026-10-07T09:14:05.2Z","ms":1834,"customer":4410}"#,
        r#"{"level":30,"time":1791105246000,"pid":1,"hostname":"orders-api-7c9d","req":{"method":"POST","url":"/api/orders"},"res":{"statusCode":201},"msg":"request completed"}"#,
        "panic: runtime error: index out of range [3] with length 3",
        r#"{"level":"error","ts":1791105247.9,"caller":"pay/client.go:120","msg":"payments-svc refused the connection","error":"dial tcp 10.96.14.2:443: connect: connection refused"}"#,
        r#"{"level":"debug","ts":1791105248.1,"caller":"gc/gc.go:9","msg":"gc: heap 182 MiB -> 96 MiB"}"#,
        r#"{"level":"info","ts":1791105249.3,"msg":"shutting down","signal":"SIGTERM"}"#,
    ];
    let start: Timestamp = "2026-10-07T09:14:03.120Z".parse().expect("a timestamp");
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let ts = start
                .checked_add(jiff::SignedDuration::from_millis(i as i64 * 731))
                .expect("in range");
            LogLine::new(ts, "orders-api-7c9d", "app", *text)
        })
        .collect()
}

/// The log: a server starting, a slow request, a failure, debug noise and one long line.
fn log() -> Vec<LogLine> {
    let texts = [
        "INFO  starting server on :8080 (version 1.42.0, commit 3f9c2ab)",
        "INFO  connected to postgres://orders-db:5432/orders",
        "DEBUG cache warmed: 12 480 keys in 412ms",
        "INFO  GET /healthz 200 0.4ms",
        "WARN  GET /api/orders 200 1834ms: slow query on orders_by_customer",
        "INFO  POST /api/orders 201 22ms",
        "ERROR POST /api/payments 502: upstream payments-svc refused the connection (dial tcp 10.96.14.2:443: connect: connection refused)",
        "INFO  retrying payments-svc in 2s (attempt 2 of 5)",
        "level=error msg=\"payment declined\" order=8812 customer=4410 reason=insufficient_funds trace=6c2f1d0a9b8e7f3c4d5e6a7b8c9d0e1f span=4f3e2d1c0b9a8f7e retry=false region=eu-west-1 cluster=prod-eu shard=7",
        "INFO  GET /api/orders/8812 200 9ms",
        "DEBUG gc: heap 182 MiB -> 96 MiB in 3.1ms",
        "INFO  shutting down: SIGTERM received",
    ];
    let start: Timestamp = "2026-10-07T09:14:03.120Z".parse().expect("a timestamp");
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let ts = start
                .checked_add(jiff::SignedDuration::from_millis(i as i64 * 731))
                .expect("in range");
            LogLine::new(ts, "orders-api-7c9d", "app", *text)
        })
        .collect()
}

fn pod() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "orders-api-7c9d", "namespace": "shop"},
        "spec": {"containers": [{"name": "app"}]},
        "status": {"phase": "Running"}
    }))
    .expect("a pod")
}

/// Renders the view, wrapped or not, with or without timestamps, in the dark or `light` theme.
/// With `json` the stream is [`json_log`], the `debug` chip is off and the failure line (seq 7)
/// is expanded.
fn render(wrap: bool, timestamps: bool, light: bool, json: bool) -> anyhow::Result<RgbaImage> {
    render_with(wrap, timestamps, light, json, None, false)
}

/// [`render`] with the search bar open on `search` (a pattern and the mode), the first match
/// being the current one; `picked` marks the failing line and selects three lines above it
/// (E08-S06).
fn render_with(
    wrap: bool,
    timestamps: bool,
    light: bool,
    json: bool,
    search: Option<(&str, SearchMode)>,
    picked: bool,
) -> anyhow::Result<RgbaImage> {
    let context = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &context);
    let entry = ClusterContext::new(cluster.clone(), context, SourceId("kubeconfig".into()));
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let ports = connector.ports_for(&cluster);
    ports.resources.insert(pod());
    ports
        .logs
        .script()
        .stream_logs
        .push_ok(Timeline::immediate(if json { json_log() } else { log() }));
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions =
        ClusterSessionManager::new(connector, source, Arc::new(FakeClockPort::default()));
    futures::executor::block_on(sessions.connect(&cluster)).expect("connect");
    let target = ResourceRef::namespaced(
        cluster,
        Gvk::new("", "v1", "Pod"),
        "shop",
        "orders-api-7c9d",
    );

    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(height(json))), |_, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        let tokens = if light {
            oxikube_ui::Tokens::light()
        } else {
            oxikube_ui::Tokens::dark()
        };
        oxikube_ui::set_tokens(cx, tokens);
        oxikube_runtime::init_deterministic(cx);
        cx.set_reduce_motion(true);
        let service = Arc::new(LogService::new(
            log_runtime(ports.logs.clock().clone(), cx),
            LogConfig::default(),
        ));
        let deps = LogViewDeps {
            service,
            sessions,
            dispatcher: Rc::new(Ignore),
            fs: Arc::new(oxikube_testkit::FakeFsPort::new()),
        };
        cx.new(|cx| LogView::new(target, None, deps, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |view, window, cx| {
        let view = view.downcast::<LogView>().expect("the root view");
        view.update(cx, |view, cx| {
            if wrap {
                view.toggle_wrap(cx);
            }
            if timestamps {
                view.toggle_timestamps(cx);
            }
            if json {
                view.toggle_level(LevelChip::Debug, cx);
                view.toggle_expanded(7, cx);
            }
            if let Some((pattern, mode)) = search {
                view.find(Some(pattern), window, cx);
                if mode == SearchMode::Filter {
                    view.toggle_filter_mode(cx);
                } else {
                    view.next_match(cx);
                }
            }
            if picked {
                view.click_line(6, false, cx);
                view.toggle_mark(cx);
                view.click_line(2, false, cx);
                view.click_line(4, true, cx);
            }
        });
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

/// The window's height for a scene.
fn height(json: bool) -> f32 {
    if json { JSON_HEIGHT } else { HEIGHT }
}

/// Checks `image` against the golden `name`.
fn check(name: &str, json: bool, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, height(json) as u32 * scale),
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
    let results = [
        render(false, true, false, false).and_then(|image| check("log_view_levels", false, image)),
        render(true, false, false, false).and_then(|image| check("log_view_wrapped", false, image)),
        render(false, true, true, false).and_then(|image| check("log_view_light", false, image)),
        render(false, false, false, true).and_then(|image| check("log_view_json", true, image)),
        render_with(
            false,
            true,
            false,
            false,
            Some(("error|warn", SearchMode::Highlight)),
            false,
        )
        .and_then(|image| check("log_view_search", false, image)),
        render_with(
            false,
            true,
            false,
            false,
            Some(("error|warn", SearchMode::Filter)),
            false,
        )
        .and_then(|image| check("log_view_filter", false, image)),
        render_with(false, true, false, false, None, true)
            .and_then(|image| check("log_view_selection_marks", false, image)),
    ];
    let mut failed = false;
    for result in results {
        if let Err(err) = result {
            eprintln!("log view screenshot failed: {err:#}");
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        println!("log view screenshots: ok");
        ExitCode::SUCCESS
    }
}
