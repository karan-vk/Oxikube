//! Kind integration for E04-S01: 2 000 pods listed in pages. Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::collections::HashSet;
use std::time::Instant;

use oxikube_domain::ids::Gvk;
use oxikube_kube::ResourcesConfig;
use oxikube_ports::{ListOptions, ResourceReader};
use oxikube_testkit::integration::TestNamespace;

use common::resources::{adapter, adapter_with, create_pods, pending_pod};

/// Pods to create; `OXIKUBE_LIST_PODS` (a multiple of 250) scales it for the 10 k budget run.
fn pod_count() -> usize {
    std::env::var("OXIKUBE_LIST_PODS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

const PAGE: u32 = 250;

fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// Resident set size of this process in MiB (`ps`, which is on macOS and Linux alike).
fn rss_mib() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .map_or(0.0, |kib| kib / 1024.0)
}

#[tokio::test]
async fn two_thousand_pods_list_in_eight_pages_without_duplicates() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let pods_wanted = pod_count();
    let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
    let client = kind.admin_client().await;

    let started = Instant::now();
    let pods = (0..pods_wanted)
        .map(|i| pending_pod(&format!("page-{i:04}"), &[("app", "paged")]))
        .collect();
    create_pods(&client, ns.name(), pods).await;
    eprintln!("created {pods_wanted} pods in {:?}", started.elapsed());

    let resources = adapter(&client);
    let options = ListOptions::default().limit(PAGE);

    // Page by page through the port: the contract callers (S02's feed) rely on.
    let before = rss_mib();
    let started = Instant::now();
    let mut names = HashSet::new();
    let mut pages = 0;
    let mut token: Option<String> = None;
    let mut first_page = None;
    let mut resource_version = None;
    loop {
        let mut request = options.clone();
        request.continue_token = token.take();
        let page = resources
            .list(&pod_gvk(), Some(ns.name()), &request)
            .await
            .expect("list page");
        first_page.get_or_insert_with(|| started.elapsed());
        pages += 1;
        assert!(page.items.len() <= PAGE as usize);
        for pod in &page.items {
            assert!(
                names.insert(pod.name().to_owned()),
                "duplicate {}",
                pod.name()
            );
            assert!(
                pod.get("/metadata/managedFields").is_none(),
                "stripped on list"
            );
            assert_eq!(pod.get_str("/status/phase"), Some("Pending"));
        }
        // Every page of one list reads the same snapshot.
        resource_version = match (&resource_version, &page.resource_version) {
            (Some(seen), Some(now)) => {
                assert_eq!(seen, now, "pages must share one resource version");
                resource_version
            }
            (None, now) => now.clone(),
            (seen, None) => seen.clone(),
        };
        if page.has_more() {
            assert!(
                page.remaining_item_count.is_some(),
                "server estimate on a partial page"
            );
            token = page.continue_token;
        } else {
            break;
        }
    }
    eprintln!(
        "paged list: first page in {:?}, {pages} pages in {:?}, RSS {before:.0} -> {:.0} MiB",
        first_page.unwrap(),
        started.elapsed(),
        rss_mib()
    );
    assert_eq!(pages, pods_wanted / PAGE as usize, "pages of {PAGE}");
    assert_eq!(names.len(), pods_wanted);
    assert!(resource_version.is_some());

    // list_all gets the same objects, and a repeat is stable.
    let all = resources
        .list_all(&pod_gvk(), Some(ns.name()), &options)
        .await
        .expect("list_all");
    let all_names: HashSet<_> = all.items.iter().map(|p| p.name().to_owned()).collect();
    assert_eq!(all.items.len(), pods_wanted);
    assert_eq!(all_names, names);
    assert_eq!(all.continue_token, None);
    assert!(all.resource_version.is_some());

    // The default page size (500) gives the same answer.
    let default = adapter_with(&client, ResourcesConfig::default())
        .list_all(&pod_gvk(), Some(ns.name()), &ListOptions::default())
        .await
        .expect("list_all default page size");
    assert_eq!(default.items.len(), pods_wanted);

    // The same list through a label selector, paged: the subset is all of them.
    let labelled = resources
        .list_all(
            &pod_gvk(),
            Some(ns.name()),
            &ListOptions::default().labels("app=paged").limit(PAGE),
        )
        .await
        .expect("selector list_all");
    assert_eq!(labelled.items.len(), pods_wanted);
}
