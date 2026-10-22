//! Tests of the charts catalog over the testkit's `FakeHelmPort`.

use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{
    ChartHit, ChartPart, ChartRef, HelmCapabilities, HelmVersion, RepoEntry,
};
use oxikube_testkit::{FakeClockPort, FakeHelmPort, HelmCall};

use super::commands::run;
use super::{ChartsService, Readiness};
use crate::HelmReposService;

fn hit(name: &str, version: &str) -> ChartHit {
    ChartHit {
        chart: ChartRef::new(name, version).in_repo("bitnami"),
        app_version: Some("1".into()),
        description: format!("{name} chart"),
    }
}

fn setup() -> (Arc<FakeHelmPort>, HelmReposService, ChartsService) {
    let fake = Arc::new(FakeHelmPort::with_capabilities(HelmCapabilities::full(
        HelmVersion::new(3, 14, 0),
    )));
    let repos = HelmReposService::new(
        fake.clone(),
        fake.clone(),
        Arc::new(FakeClockPort::default()),
    );
    let charts = ChartsService::new(fake.clone(), repos.catalog_changes());
    (fake, repos, charts)
}

fn searches(fake: &FakeHelmPort) -> usize {
    fake.recorded_calls()
        .iter()
        .filter(|c| matches!(c, HelmCall::Search(_)))
        .count()
}

fn bitnami() -> RepoEntry {
    RepoEntry {
        name: "bitnami".into(),
        url: "https://x".into(),
    }
}

#[tokio::test]
async fn search_reads_the_catalog_once_and_filters_every_keyword_in_memory() {
    let (fake, _, charts) = setup();
    fake.script().search.push_ok(vec![
        hit("nginx", "9.0.0"),
        hit("nginx", "15.0.0"),
        hit("redis", "1.0.0"),
    ]);
    let found = charts.search("Nginx ").await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].latest().version, "15.0.0");
    assert_eq!(found[0].versions.len(), 2);
    // Every keyword after that (any case, by name, repository or description) is filtered from
    // the cached catalog: nothing else is scripted, and `helm` ran once.
    assert_eq!(charts.search("redis").await.unwrap().len(), 1);
    assert_eq!(charts.search("BITNAMI/").await.unwrap().len(), 2);
    assert_eq!(charts.search("redis chart").await.unwrap()[0].name, "redis");
    assert!(charts.search("zzz").await.unwrap().is_empty());
    let all = charts.search("").await.unwrap();
    assert!(Arc::ptr_eq(&all, &charts.search("  ").await.unwrap()));
    assert_eq!(searches(&fake), 1);
    let HelmCall::Search(query) = &fake.recorded_calls()[0] else {
        panic!("search");
    };
    assert!(query.all_versions, "the detail picker needs every version");
    assert!(query.keyword.is_empty(), "the whole catalog is read once");
}

/// The audit measured each keystroke's `helm search repo <kw> --versions` at 0.9-1.2 s and
/// ~200 MB of child RSS (8 repos, 7,002 chart versions). Filtering the same catalog in memory
/// takes well under a millisecond in release; the bound here is loose for debug runners.
#[tokio::test]
async fn filtering_a_large_catalog_is_fast() {
    let (fake, _, charts) = setup();
    let hits = (0..7_000)
        .map(|i| hit(&format!("chart-{}", i / 5), &format!("1.{}.0", i % 5)))
        .collect();
    fake.script().search.push_ok(hits);
    assert_eq!(charts.search("").await.unwrap().len(), 1_400);
    let started = std::time::Instant::now();
    for keyword in ["c", "ch", "cha", "chart-1", "chart-13", "chart-139"] {
        charts.search(keyword).await.unwrap();
    }
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_millis(500),
        "six keywords took {took:?}"
    );
    assert_eq!(charts.search("chart-139").await.unwrap().len(), 11);
    assert_eq!(searches(&fake), 1);
}

#[tokio::test]
async fn a_repository_change_drops_the_cache() {
    let (fake, repos, charts) = setup();
    fake.script().search.push_ok(vec![hit("nginx", "1.0.0")]);
    fake.script().search.push_ok(vec![hit("nginx", "2.0.0")]);
    assert_eq!(
        charts.search("").await.unwrap()[0].latest().version,
        "1.0.0"
    );
    fake.script().repos.push_ok(vec![RepoEntry {
        name: "bitnami".into(),
        url: "https://x".into(),
    }]);
    repos.update(&[]).await.ok();
    fake.script().repos.push_ok(vec![RepoEntry {
        name: "bitnami".into(),
        url: "https://x".into(),
    }]);
    repos
        .add(oxikube_domain::helm::RepoAddRequest::new(
            "other",
            "https://o",
        ))
        .await
        .ok();
    assert_eq!(
        charts.search("").await.unwrap()[0].latest().version,
        "2.0.0"
    );
}

#[tokio::test]
async fn detail_is_cached_per_chart_version_and_part() {
    let (fake, _, charts) = setup();
    fake.script().repos.push_ok(vec![bitnami()]);
    fake.script().show.push_ok("# README one".to_owned());
    fake.script().show.push_ok("replicas: 1".to_owned());
    let chart = ChartRef::new("nginx", "1.0.0").in_repo("bitnami");
    let readme = charts.detail(&chart, ChartPart::Readme).await.unwrap();
    assert_eq!(&*readme, "# README one");
    let again = charts.detail(&chart, ChartPart::Readme).await.unwrap();
    assert!(Arc::ptr_eq(&readme, &again));
    let values = charts.detail(&chart, ChartPart::Values).await.unwrap();
    assert_eq!(&*values, "replicas: 1");
    let shows = fake
        .recorded_calls()
        .iter()
        .filter(|c| matches!(c, HelmCall::Show(..)))
        .count();
    assert_eq!(shows, 2);
}

#[tokio::test]
async fn detail_needs_a_repository() {
    let (_, _, charts) = setup();
    let error = charts
        .detail(&ChartRef::new("nginx", ""), ChartPart::Readme)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
}

#[tokio::test]
async fn readiness_distinguishes_no_helm_no_repos_and_ready() {
    let (fake, _, charts) = setup();
    fake.script().repos.push_ok(Vec::new());
    assert_eq!(charts.ready().await.unwrap(), Readiness::NoRepos);
    let repo = RepoEntry {
        name: "bitnami".into(),
        url: "https://x".into(),
    };
    fake.script().repos.push_ok(vec![repo.clone()]);
    assert_eq!(charts.ready().await.unwrap(), Readiness::Ready(vec![repo]));
    fake.set_capabilities(HelmCapabilities::native_only());
    assert_eq!(charts.ready().await.unwrap(), Readiness::NoHelm);
}

#[tokio::test]
async fn the_commands_search_show_and_hand_over_the_install() {
    let (fake, _, charts) = setup();
    fake.script()
        .search
        .push_ok(vec![hit("nginx", "1.0.0"), hit("nginx", "2.0.0")]);
    let out = run(
        &charts,
        Command::HelmSearchCharts {
            keyword: "ng".into(),
        },
    )
    .await
    .unwrap();
    let data = out.data.unwrap();
    assert_eq!(data["charts"][0]["latest"], "2.0.0");
    assert_eq!(data["charts"][0]["versions"].as_array().unwrap().len(), 2);

    fake.script().repos.push_ok(vec![bitnami()]);
    fake.script().show.push_ok("readme".to_owned());
    let out = run(
        &charts,
        Command::HelmShowChart {
            repo: "bitnami".into(),
            name: "nginx".into(),
            version: None,
            part: ChartPart::Readme,
        },
    )
    .await
    .unwrap();
    assert_eq!(out.data.unwrap()["text"], "readme");

    let mut intents = charts.install_requests();
    assert!(intents.borrow().is_none());
    run(
        &charts,
        Command::HelmInstallChart {
            repo: "bitnami".into(),
            name: "nginx".into(),
            version: Some("2.0.0".into()),
        },
    )
    .await
    .unwrap();
    assert!(intents.has_changed().unwrap());
    let chart = intents.borrow_and_update().clone().unwrap();
    assert_eq!(chart.reference(), "bitnami/nginx");
    assert_eq!(chart.version, "2.0.0");
    assert!(run(&charts, Command::HelmOpenCharts).await.is_err());
}

#[tokio::test]
async fn without_helm_search_is_unsupported() {
    let (fake, _, charts) = setup();
    fake.set_capabilities(HelmCapabilities::native_only());
    let error = charts.search("x").await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn show_refuses_urls_paths_and_unconfigured_repositories() {
    let (fake, _, charts) = setup();
    fake.script().repos.push_ok(vec![bitnami()]);
    let show = |repo: &str, name: &str| Command::HelmShowChart {
        repo: repo.into(),
        name: name.into(),
        version: None,
        part: ChartPart::Values,
    };
    for (repo, name) in [
        // `https://attacker.example/x.tgz` once joined.
        ("https:", "/attacker.example/x.tgz"),
        // A local chart directory.
        ("/home/user", "some-chart-dir"),
        ("bitnami", "../../etc"),
        ("bitnami", "x/y"),
        ("bitnami", ".hidden"),
        ("bitnami", ""),
        // Not a configured repository (even after reading Helm's list again).
        ("charts", "nginx"),
    ] {
        fake.script().repos.push_ok(vec![bitnami()]);
        let error = run(&charts, show(repo, name)).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{repo}/{name}");
    }
    assert!(
        !fake
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, HelmCall::Show(..))),
        "helm show never ran"
    );
    fake.script().show.push_ok("replicas: 1".to_owned());
    assert!(run(&charts, show("bitnami", "nginx")).await.is_ok());
}
