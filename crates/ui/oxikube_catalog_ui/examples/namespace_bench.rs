//! Cost of the namespace selector's local work, for the PR's perf numbers (ADR 0013: input costs
//! one frame, the search filters locally).
//!
//! ```text
//! cargo run -p oxikube_catalog_ui --profile release-fast --example namespace_bench
//! ```
//!
//! It times, over a 10 000-namespace cluster (the largest we plan for) and nine favourites:
//! the rows for an empty query, for a one-letter query and for a selective query (what a
//! keystroke in the search box costs), and a selection change through the `NamespaceService`
//! (session update + debounce bookkeeping + remembering, on in-memory fakes).

#![allow(clippy::print_stdout)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_app::ClusterSessionManager;
use oxikube_app::session::namespaces::{NamespaceCatalog, NamespaceService, NamespaceSource};
use oxikube_catalog_ui::namespaces::build_rows;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::{NamespaceFavourites, NamespaceSelection};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

const NAMESPACES: usize = 10_000;
const ROUNDS: u32 = 200;

fn time<R>(label: &str, rounds: u32, mut f: impl FnMut() -> R) {
    let mut samples: Vec<Duration> = (0..rounds)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(f());
            start.elapsed()
        })
        .collect();
    samples.sort();
    let p = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<46} p50 {:>9.1?}  p95 {:>9.1?}  max {:>9.1?}",
        p(0.5),
        p(0.95),
        p(1.0)
    );
}

fn main() {
    let mut catalog = NamespaceCatalog::unlisted();
    catalog.source = NamespaceSource::Cluster;
    catalog.names = (0..NAMESPACES)
        .map(|i| format!("team-{i:05}-prod"))
        .collect();
    let favourites: NamespaceFavourites = catalog
        .names
        .iter()
        .step_by(1_111)
        .take(9)
        .map(String::as_str)
        .collect();
    let selection = NamespaceSelection::from_names(catalog.names.iter().take(3));

    println!(
        "{NAMESPACES} namespaces, {} favourites, {ROUNDS} rounds",
        favourites.len()
    );
    time("rows, empty query", ROUNDS, || {
        build_rows("", &catalog, &selection, &favourites)
    });
    time("rows, query \"t\" (matches all)", ROUNDS, || {
        build_rows("t", &catalog, &selection, &favourites)
    });
    time("rows, query \"05-pr\" (selective)", ROUNDS, || {
        build_rows("00005-pr", &catalog, &selection, &favourites)
    });

    // A selection change through the service, on fakes.
    let context = ContextName::new("bench");
    let cluster = ClusterId::new("/bench", &context);
    let entry = ClusterContext {
        cluster: cluster.clone(),
        context,
        source: SourceId("bench".into()),
        server: None,
        default_namespace: None,
        cluster_name: None,
        user: None,
        problem: None,
    };
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()])),
        clock.clone(),
    );
    manager.open(&entry, Default::default());
    let service = NamespaceService::new(manager, Arc::new(FakeStatePort::new()), clock);
    let mut flip = false;
    time("service.select (session + remember)", ROUNDS, || {
        flip = !flip;
        let selection = NamespaceSelection::single(if flip { "a" } else { "b" });
        futures::executor::block_on(service.select(&cluster, selection)).expect("select")
    });
}
