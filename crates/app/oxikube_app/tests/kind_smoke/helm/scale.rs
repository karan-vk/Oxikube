//! Epic done-when item 4 on a real API server (E16-S12): 500 releases list natively within the
//! budget. The release records are written as Helm stores them (one `kubectl create` of a
//! `List`, not 500 installs), one in five with the ConfigMap driver. The time prints as a
//! `perf-jsonl` line, which the integration workflow keeps.

use std::time::Instant;

use oxikube_domain::session::NamespaceSelection;
use oxikube_helm::HelmCliConfig;
use oxikube_ports::HelmPort as _;
use oxikube_testkit::integration::helm::{Driver, release_record};
use oxikube_testkit::integration::{TestNamespace, events};
use serde_json::json;

use super::World;

const RELEASES: usize = 500;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_hundred_releases_list_natively_within_the_budget() {
    let Some(w) = World::connect(HelmCliConfig::default()).await else {
        return;
    };
    let ns = TestNamespace::create(w.context()).expect("namespace");
    let namespace = ns.name();
    let items: Vec<_> = (0..RELEASES)
        .map(|n| {
            let driver = if n % 5 == 0 {
                Driver::ConfigMap
            } else {
                Driver::Secret
            };
            release_record(driver, namespace, &format!("rel-{n:03}"), 1)
        })
        .collect();
    events::create(
        w.context(),
        namespace,
        &json!({"apiVersion": "v1", "kind": "List", "items": items}),
    )
    .expect("create the release records");

    let native = w.native();
    let selection = NamespaceSelection::single(namespace);
    let started = Instant::now();
    let releases = native
        .list_releases(&w.cluster, &selection)
        .await
        .expect("list");
    let cold = started.elapsed();
    assert_eq!(releases.len(), RELEASES);
    let started = Instant::now();
    native
        .list_releases(&w.cluster, &selection)
        .await
        .expect("list");
    let warm = started.elapsed();
    eprintln!(
        "perf-jsonl {}",
        json!({
            "suite": "helm", "scenario": "native_list_500",
            "cold_ms": cold.as_millis(), "warm_ms": warm.as_millis(),
            "debug": cfg!(debug_assertions),
        })
    );
    // The budget (1 s) is for release builds; an unoptimised build gets three times the room.
    // CI enforces the 1 s bound: the `kind` job reruns this test with `--release`.
    let budget_ms = if cfg!(debug_assertions) { 3000 } else { 1000 };
    assert!(
        cold.as_millis() < budget_ms,
        "cold list of {RELEASES} took {cold:?}"
    );
    assert!(
        warm.as_millis() < budget_ms,
        "warm list of {RELEASES} took {warm:?}"
    );
}
