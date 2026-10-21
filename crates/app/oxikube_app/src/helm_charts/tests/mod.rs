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

#[tokio::test]
async fn search_groups_versions_newest_first_and_caches_per_keyword() {
    let (fake, _, charts) = setup();
    fake.script().search.push_ok(vec![
        hit("nginx", "9.0.0"),
        hit("nginx", "15.0.0"),
        hit("redis", "1.0.0"),
    ]);
    let found = charts.search("Nginx ").await.unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].latest().version, "15.0.0");
    // The same keyword (any case) is served from the cache: nothing else is scripted.
    let again = charts.search("nginx").await.unwrap();
    assert!(Arc::ptr_eq(&found, &again));
    assert_eq!(searches(&fake), 1);
    let HelmCall::Search(query) = &fake.recorded_calls()[0] else {
        panic!("search");
    };
    assert!(query.all_versions, "the detail picker needs every version");
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
