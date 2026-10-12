//! The section matrix on a real cluster (E12-S20, the epic's safety net): one object of every
//! kind of `oxikube_testkit::sections` seeded into kind, and the app, on its real init order with
//! the kind context connected, checked kind by kind the way a user reaches it.
//!
//! For each kind: its sidebar entry opens its table, which lists the seeded object with the
//! documented columns (the Namespace column left out, one namespace being selected) and draws
//! it; the object's detail opens in the drawer, reads the object from the API server and draws
//! with the kind's own panel; the row menu offers the kind's documented actions; and each
//! mutating one, dispatched on the app's bus as its row sends it, asks for its declared tier and
//! is refused on a read-only cluster, while the object stays as it was. (The mutations
//! themselves, run to the end, are `oxikube_app`'s `kind_smoke` suites and `kind_workloads`:
//! scale, restart, rollback, cordon, drain, evict, trigger, suspend.) The custom resources
//! section adds the printer-column parity: a Widget table shows the columns `kubectl get` does.
//!
//! | Module | Holds |
//! |---|---|
//! | `cluster` | seeding the cases on the cluster (and deleting the cluster-scoped seeds), the server's Table columns |
//! | `app` | driving the app: a kind's table from its sidebar entry, its detail, a command on the bus |
//! | `check` | the checks of one kind |
//!
//! Epic E12's "done when", and where each is proven:
//!
//! | Criterion | Proof |
//! |---|---|
//! | 1. every kind has columns, a detail panel and its actions | this suite and `mount::tests::sections` (fakes) over `oxikube_testkit::sections` |
//! | 2. the k9s verbs are mapped | `startup::tests::k9s_verbs` |
//! | 3. mutations respect read-only and their tiers | `mount::tests::sections::guard` (every mutating row action: tier, audit, read-only, tool stub), and this suite against the cluster |
//! | 4. a CRD's columns are `kubectl get`'s | `check::printer_columns` here, `oxikube_app` `kind_smoke::columns` |
//! | 5. scale, restart, rollback, cordon, drain, evict, trigger, suspend run on kind | `oxikube_app` `kind_smoke::{scale, rollout, rollback, nodes, drain, pods, workloads}`, `kind_workloads` |
//!
//! One test per sidebar section, so a run shards by section and a failure names it. The optional
//! kinds (Gateway API, VerticalPodAutoscaler, snapshots) are skipped, saying so, on a cluster that
//! does not serve them. Nothing touches the node: the Node case reads the cluster's own node, and
//! every seed is inert (see `oxikube_testkit::sections`). Every namespaced object lives in the
//! test's `oxi-test-<rand>` namespace and every cluster-scoped one carries its name; both go when
//! the test ends, also when it fails.
//!
//! `cargo test -p oxikube --features integration --test kind_sections` with
//! `OXIKUBE_TEST_CONTEXT` set (`cargo xtask kind-up`); without it each test returns at once.
#![cfg(feature = "integration")]
// The run says which optional kinds it skipped and how long each section took.
#![allow(clippy::print_stdout)]

#[path = "../kind_common/mod.rs"]
mod kind_common;

mod app;
mod check;
mod cluster;

use std::time::Instant;

use gpui::TestAppContext;
use oxikube::app_state::AppState;
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_testkit::sections::{KindCase, Seed, catalog};

use kind_common::{Launched, launch};

/// Nothing more.
fn nothing(_: &mut Launched, _: &str) {}

/// Seeds `sections`' kinds, starts the app on the kind context with the test's namespace
/// selected, checks each kind, then runs `more` on the same app.
fn run(cx: &mut TestAppContext, sections: &[&str], more: fn(&mut Launched, &str)) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let started = Instant::now();

    let ns = TestNamespace::create(&context).expect("namespace");
    let seed = Seed {
        namespace: ns.name(),
        stem: ns.name(),
    };
    let cases: Vec<&KindCase> = catalog()
        .filter(|case| sections.contains(&case.section))
        .collect();
    let seeded = cluster::seed(&context, &seed, &cases);

    let mut app = launch(cx, &context);
    let state = app.vcx.read(AppState::global);
    state
        .services()
        .sessions
        .set_namespace_selection(&app.cluster, NamespaceSelection::single(ns.name()))
        .expect("the session is open");
    for case in &cases {
        match seeded.name_of(case) {
            Some(name) => check::kind(&mut app, &context, case, &name),
            None => println!(
                "skipping {}: {}/{} is not served by this cluster",
                case.kind, case.group, case.version
            ),
        }
    }
    more(&mut app, &context);
    println!(
        "section matrix {sections:?}: {} kinds in {:?}",
        cases.len(),
        started.elapsed()
    );
    drop(app);
    drop(seeded);
}

#[gpui::test]
fn nodes_and_namespaces(cx: &mut TestAppContext) {
    run(cx, &["nodes", "namespaces"], nothing);
}

#[gpui::test]
fn workloads(cx: &mut TestAppContext) {
    run(cx, &["workloads"], nothing);
}

#[gpui::test]
fn config(cx: &mut TestAppContext) {
    run(cx, &["config"], nothing);
}

#[gpui::test]
fn network(cx: &mut TestAppContext) {
    run(cx, &["network"], nothing);
}

#[gpui::test]
fn storage(cx: &mut TestAppContext) {
    run(cx, &["storage"], nothing);
}

#[gpui::test]
fn custom_resources(cx: &mut TestAppContext) {
    run(cx, &["custom-resources"], check::printer_columns);
}
