//! Micro benchmark of the `:` jump bar over a cluster with about 2 400 alias names (the built-in
//! k9s names, the stock types and 550 CRDs): opening it (construction and first frame), a
//! keystroke (parse, match the word under the caret, the frame after), parsing and planning a
//! line, and a redraw.
//!
//! `cargo run -p oxikube_palette --features test-support --profile release-fast --example jump_bench`
//!
//! Runs on GPUI's test platform (no GPU, test text system), so it measures what the bar adds per
//! frame. It is a regression check against docs/PERFORMANCE.md (palette: open <= 1 frame, a
//! completion filter over 2 000 candidates <= 5 ms, keystroke-to-visible <= 1 frame), not the
//! frame budget itself (that needs `oxikube --perf`).

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube_app::search::aliases::AliasRegistry;
use oxikube_app::search::jump::{self, JumpContext, JumpEnv};
use oxikube_app::session::namespaces::{NamespaceCatalog, NamespaceService};
use oxikube_app::{ClusterCatalog, ClusterSessionManager};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_keymap::KeymapOptions;
use oxikube_palette::jump::{JumpBar, JumpDelegate, JumpHost, JumpRequest, JumpSources};
use oxikube_testkit::kinds::{KindSpec, core_kinds};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::{CommandDispatcher, Workspace, test_support::open_workspace};

const CRDS: usize = 550;

fn cluster() -> ClusterId {
    ClusterId::new("bench", &ContextName::new("bench"))
}

struct Sink;

impl CommandDispatcher for Sink {
    fn dispatch(&self, _: oxikube_domain::command::Command, _: &mut gpui::App) {}
}

struct World {
    vcx: VisualTestContext,
    workspace: Entity<Workspace>,
    host: Rc<JumpHost>,
    aliases: AliasRegistry,
}

fn world(cx: &mut TestAppContext) -> World {
    let (workspace, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| {
        oxikube_runtime::init_deterministic(cx);
        oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
    });
    let clock = Arc::new(FakeClockPort::default());
    let source = Arc::new(FakeClusterSourcePort::new());
    let state = Arc::new(FakeStatePort::new());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        source.clone(),
        clock.clone(),
    );
    let aliases = AliasRegistry::new();
    let mut kinds = core_kinds();
    kinds.extend((0..CRDS).map(|i| {
        KindSpec::new(
            &format!("group{}.example.io", i % 40),
            "v1",
            &format!("Thing{i}"),
            &format!("things{i}"),
        )
        .short(&format!("t{i}"))
        .build()
    }));
    aliases.table(&cluster()).set_discovered(&kinds);
    let sources = JumpSources {
        active: Rc::new(|_| Some(cluster())),
        sessions: sessions.clone(),
        catalog: ClusterCatalog::new(source, state.clone(), clock.clone()),
        aliases: aliases.clone(),
        namespaces: NamespaceService::new(sessions, state, clock),
    };
    let host = Rc::new(JumpHost::new(&workspace, Rc::new(Sink), sources));
    vcx.update(|window, cx| host.install(window, cx));
    vcx.run_until_parked();
    World {
        vcx,
        workspace,
        host,
        aliases,
    }
}

fn open(w: &mut World) -> f64 {
    let host = w.host.clone();
    let started = Instant::now();
    w.vcx
        .update(|window, cx| host.apply(JumpRequest::Open, window, cx));
    w.vcx.run_until_parked();
    w.vcx.update(|window, cx| window.draw(cx).clear(cx));
    started.elapsed().as_secs_f64() * 1000.0
}

fn close(w: &mut World) {
    let host = w.host.clone();
    w.vcx
        .update(|window, cx| host.apply(JumpRequest::Open, window, cx));
    w.vcx.run_until_parked();
}

fn picker(w: &mut World) -> Entity<oxikube_palette::Picker<JumpDelegate>> {
    let workspace = w.workspace.clone();
    w.vcx.update(|_, cx| {
        workspace
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<JumpBar>()
            .expect("the bar is open")
            .read(cx)
            .picker()
            .clone()
    })
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.total_cmp(b));
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    println!(
        "jump_bench {name}: {} runs, ms mean {mean:.3} p50 {:.3} p95 {:.3} max {:.3}",
        ms.len(),
        percentile(&ms, 50),
        percentile(&ms, 95),
        ms[ms.len() - 1]
    );
}

struct Env(oxikube_app::search::aliases::AliasTable, Vec<JumpContext>);

impl JumpEnv for Env {
    fn active_cluster(&self) -> Option<ClusterId> {
        Some(cluster())
    }
    fn contexts(&self) -> &[JumpContext] {
        &self.1
    }
    fn aliases(&self, _: &ClusterId) -> oxikube_app::search::aliases::AliasTable {
        self.0.clone()
    }
    fn namespaces(&self, _: &ClusterId) -> Option<NamespaceCatalog> {
        None
    }
}

fn main() {
    // Opening: construction, the aliases of the cluster and the first frame.
    let mut opens = Vec::new();
    for _ in 0..15 {
        let mut cx = TestAppContext::single();
        let mut w = world(&mut cx);
        opens.push(open(&mut w));
    }
    println!(
        "jump_bench open (cold, first window of the process): {:.3} ms",
        opens[0]
    );
    report("open", opens);

    let mut cx = TestAppContext::single();
    let mut w = world(&mut cx);
    let names = w.aliases.table(&cluster()).len();
    println!("jump_bench alias names in the cluster: {names}");
    open(&mut w);

    // A keystroke: the line changes, the matches arrive, the frame after.
    let queries = [
        "t",
        "t1",
        "t12",
        "t123",
        "deploy",
        "deploy ",
        "deploy kube",
        "things5",
        "zzz",
        "",
        "cert",
        "pod /re app=x",
    ];
    let (mut keystroke, mut frame) = (Vec::new(), Vec::new());
    for round in 0..60 {
        let query = queries[round % queries.len()];
        let picker = picker(&mut w);
        let started = Instant::now();
        w.vcx
            .update(|window, cx| picker.update(cx, |p, cx| p.set_query(query, window, cx)));
        w.vcx.run_until_parked();
        keystroke.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        w.vcx.update(|window, cx| window.draw(cx).clear(cx));
        frame.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("keystroke (query to matches in)", keystroke);
    report("keystroke (frame after)", frame);
    close(&mut w);

    // Parsing and planning a line: what each keystroke and Enter cost, off any window.
    let env = Env(w.aliases.table(&cluster()), Vec::new());
    let lines = [
        "pods",
        "deploy kube-system /api app=x @prod",
        "things17 /re",
        "podz",
    ];
    let (mut parse, mut plan) = (Vec::new(), Vec::new());
    for round in 0..2_000 {
        let line = lines[round % lines.len()];
        let started = Instant::now();
        std::hint::black_box(jump::parse(line).ok());
        parse.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        std::hint::black_box(jump::plan(line, &env).ok());
        plan.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("parse", parse);
    report("plan (alias lookup, suggestions for an unknown word)", plan);

    // A redraw of the open bar.
    open(&mut w);
    let mut redraw = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        w.vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        redraw.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    report("redraw", redraw);
}
