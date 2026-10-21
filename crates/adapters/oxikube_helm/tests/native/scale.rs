//! Epic done-when (4): a 500-release cluster lists in under a second. Each release has
//! three revisions and a large record (a ~200 KiB rendered manifest), so the list must stay
//! metadata-first and decode only the latest revision of each release.

use std::time::Instant;

use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::helm::HelmPort;
use serde_json::json;

use crate::support::{
    Driver, cluster, full_lists, gets, helm_encode, object_with_data, reader, record,
};

const RELEASES: u32 = 500;
const REVISIONS: u32 = 3;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_hundred_releases_list_within_the_budget() {
    let manifest: String = (0..4_000)
        .map(|i| format!("---\nkind: ConfigMap\nmetadata:\n  name: cm-{i}\ndata:\n  k: v{i}\n"))
        .collect();
    // Two payloads encoded once and shared (the reader keys on labels, not on the
    // record's own name), so building 1 500 large objects stays fast in debug builds.
    let payload = |status: &str| {
        let mut rec = record("rel", "ns", 1, status);
        rec["manifest"] = json!(manifest);
        helm_encode(&rec)
    };
    let (deployed, superseded) = (payload("deployed"), payload("superseded"));
    let mut objects = Vec::new();
    for n in 0..RELEASES {
        for rev in 1..=REVISIONS {
            let (status, data) = if rev == REVISIONS {
                ("deployed", &deployed)
            } else {
                ("superseded", &superseded)
            };
            let driver = if n % 5 == 0 {
                Driver::ConfigMap
            } else {
                Driver::Secret
            };
            let (name, namespace) = (format!("rel-{n:03}"), format!("ns-{}", n % 20));
            objects.push(object_with_data(
                driver, &namespace, &name, rev, status, data,
            ));
        }
    }
    let (reader, fake) = reader(objects);

    let started = Instant::now();
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let cold = started.elapsed();
    assert_eq!(releases.len(), RELEASES as usize);
    assert!(releases.iter().all(|r| r.revision == REVISIONS));
    assert_eq!(
        gets(&fake),
        RELEASES as usize,
        "one fetch per release, latest only"
    );
    assert_eq!(full_lists(&fake), 0);

    let started = Instant::now();
    reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    let warm = started.elapsed();
    eprintln!(
        "helm native list: {RELEASES} releases x {REVISIONS} revisions, {} KiB manifest each: \
         cold {cold:?}, warm {warm:?}",
        manifest.len() / 1024
    );
    // The budget is for release builds; unoptimised gzip/JSON gets three times the room.
    let budget_ms = if cfg!(debug_assertions) { 3000 } else { 1000 };
    assert!(cold.as_millis() < budget_ms, "cold list took {cold:?}");
    assert!(
        warm < cold,
        "the second list is served from the decode cache"
    );
}
