//! The page in a workspace window.

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::helm::ShowKind;
use oxikube_testkit::HelmCall;

use super::{Harness, hit, repo};
use crate::charts::{DEBOUNCE, Load, Phase};

fn nginx_and_redis() -> Vec<oxikube_domain::helm::ChartHit> {
    vec![
        hit("bitnami", "nginx", "15.0.0", "1.25.0", "A web server"),
        hit("bitnami", "nginx", "14.2.0", "1.24.0", "A web server"),
        hit("bitnami", "nginx", "9.0.0", "1.1.0", "A web server"),
        hit("jetstack", "redis", "1.0.0", "7.2", "A cache"),
    ]
}

#[gpui::test]
fn cards_show_icon_name_description_versions_and_repo(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami", "jetstack"], nginx_and_redis());
    assert!(h.is_laid_out("helm-charts"));
    assert!(h.is_laid_out("helm-chart-cards"));
    assert!(h.is_laid_out("helm-chart-card:bitnami/nginx"));
    assert!(h.is_laid_out("helm-chart-card:jetstack/redis"));
    assert!(
        h.is_laid_out("helm-chart-placeholder"),
        "nothing selected yet"
    );
    h.read(|v| {
        assert_eq!(v.phase(), &Phase::Ready { repos: 2 });
        let nginx = &v.cards()[0];
        assert_eq!(
            (
                nginx.name.as_str(),
                nginx.repo.as_str(),
                nginx.version.as_str()
            ),
            ("nginx", "bitnami", "15.0.0")
        );
        assert_eq!(nginx.app_version, "1.25.0");
        assert_eq!(nginx.versions, 3);
    });
    let HelmCall::Search(query) = h
        .fake
        .recorded_calls()
        .into_iter()
        .find(|c| matches!(c, HelmCall::Search(_)))
        .expect("searched")
    else {
        unreachable!()
    };
    assert!(query.keyword.is_empty(), "an empty query lists every chart");
}

#[gpui::test]
fn selecting_a_card_shows_readme_versions_and_default_values(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami", "jetstack"], nginx_and_redis());
    h.fake
        .script()
        .show
        .push_ok("# nginx\n\n<script>alert(1)</script>\n![logo](https://x/y.png)\n".to_owned());
    h.fake
        .script()
        .show
        .push_ok("replicaCount: 1\nimage:\n  tag: 1.25\n".to_owned());
    h.click("helm-chart-card:bitnami/nginx");
    assert!(h.is_laid_out("helm-chart-detail"));
    assert!(h.is_laid_out("helm-chart-readme"), "the README is shown");
    h.read(|v| {
        let d = v.detail().expect("detail");
        assert_eq!(d.version, "15.0.0", "the newest version first");
        assert_eq!(d.readme, Load::Ready);
        assert_eq!(d.values, Load::Idle, "values load when their tab opens");
    });
    let shown = h.fake.recorded_calls();
    assert!(
        shown
            .iter()
            .any(|c| matches!(c, HelmCall::Show(chart, ShowKind::Readme)
            if chart.reference() == "bitnami/nginx" && chart.version == "15.0.0"))
    );

    // Versions, newest first, all of them.
    h.click("helm-chart-tab:versions");
    for v in ["15.0.0", "14.2.0", "9.0.0"] {
        assert!(h.is_laid_out(&format!("helm-chart-version:{v}")), "{v}");
    }

    // Default values.
    h.click("helm-chart-tab:values");
    assert!(h.is_laid_out("helm-chart-values"));
    h.read(|v| assert_eq!(v.detail().unwrap().values, Load::Ready));
    let code = h.read(|v| v.detail().unwrap().code.clone());
    let text = h.vcx.read(|cx| code.read(cx).text().map(|t| t.to_string()));
    assert_eq!(
        text.as_deref(),
        Some("replicaCount: 1\nimage:\n  tag: 1.25\n")
    );
}

#[gpui::test]
fn the_readme_is_sanitised_before_it_is_rendered(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake
        .script()
        .show
        .push_ok("hi <script>alert(1)</script> ![x](https://t/p.png)".to_owned());
    h.click("helm-chart-card:bitnami/nginx");
    let (state, source) = h.read(|v| {
        let d = v.detail().unwrap();
        (d.readme.clone(), d.readme_text.clone())
    });
    let source = source.unwrap_or_else(|| panic!("README is {state:?}"));
    assert!(!source.contains("<script"), "{source}");
    assert!(!source.contains("!["), "{source}");
    assert!(source.contains("[x](https://t/p.png)"), "{source}");
}

#[gpui::test]
fn picking_a_version_reads_that_versions_texts(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake.script().show.push_ok("# new".to_owned());
    h.fake.script().show.push_ok("# old".to_owned());
    h.click("helm-chart-card:bitnami/nginx");
    h.click("helm-chart-tab:versions");
    h.click("helm-chart-version:9.0.0");
    h.click("helm-chart-tab:readme");
    h.read(|v| {
        let d = v.detail().unwrap();
        assert_eq!(d.version, "9.0.0");
        assert_eq!(d.readme, Load::Ready);
    });
    let versions: Vec<_> = h
        .fake
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            HelmCall::Show(chart, ShowKind::Readme) => Some(chart.version),
            _ => None,
        })
        .collect();
    assert_eq!(versions, ["15.0.0", "9.0.0"]);
}

#[gpui::test]
fn install_sends_the_install_command_with_the_chart_prefilled(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake.script().show.push_ok("# nginx".to_owned());
    assert!(
        !h.is_laid_out("helm-chart-install"),
        "nothing to install yet"
    );
    h.click("helm-chart-card:bitnami/nginx");
    h.click("helm-chart-tab:versions");
    h.click("helm-chart-version:14.2.0");
    h.click("helm-chart-install");
    assert_eq!(
        h.bus.sent(),
        [Command::HelmInstallChart {
            repo: "bitnami".into(),
            name: "nginx".into(),
            version: Some("14.2.0".into()),
        }]
    );
}

#[gpui::test]
fn a_readme_that_fails_to_load_says_why(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake
        .script()
        .show
        .push_err(OxiError::network("repo unreachable"));
    h.click("helm-chart-card:bitnami/nginx");
    assert!(h.is_laid_out("helm-chart-readme-failed"));
    h.read(|v| assert!(matches!(v.detail().unwrap().readme, Load::Failed(_))));
}

#[gpui::test]
fn an_empty_readme_is_said_so(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake.script().show.push_ok("  \n".to_owned());
    h.click("helm-chart-card:bitnami/nginx");
    assert!(h.is_laid_out("helm-chart-readme-empty"));
}

#[gpui::test]
fn typing_searches_once_after_the_pause(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake
        .script()
        .search
        .push_ok(vec![hit("bitnami", "redis", "1.0.0", "7", "A cache")]);
    let searches = |h: &Harness| {
        h.fake
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, HelmCall::Search(_)))
            .count()
    };
    h.type_query("red");
    assert_eq!(searches(&h), 1, "typing alone does not search");
    h.vcx.executor().advance_clock(DEBOUNCE);
    h.settle();
    assert_eq!(searches(&h), 2, "one search for the three keystrokes");
    h.read(|v| {
        assert_eq!(v.cards().len(), 1);
        assert_eq!(v.cards()[0].name, "redis");
    });
    let last = h
        .fake
        .recorded_calls()
        .into_iter()
        .rev()
        .find_map(|c| match c {
            HelmCall::Search(q) => Some(q),
            _ => None,
        })
        .unwrap();
    assert_eq!(last.keyword, "red");
}

#[gpui::test]
fn no_matching_chart_is_said_so(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], Vec::new());
    assert!(h.is_laid_out("helm-charts-empty"));
    assert!(!h.is_laid_out("helm-chart-cards"));
}

#[gpui::test]
fn a_failed_search_shows_the_reason_and_keeps_the_page(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake
        .script()
        .search
        .push_err(OxiError::network("no route"));
    h.type_query("zzz");
    h.vcx.executor().advance_clock(DEBOUNCE);
    h.settle();
    assert!(h.read(|v| v.problem().is_some()));
    assert!(h.is_laid_out("helm-charts-problem"));
    assert!(h.is_laid_out("helm-chart-cards"), "the last result stays");
}

#[gpui::test]
fn without_repositories_the_page_offers_the_default_ones(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &[], Vec::new());
    assert_eq!(h.read(|v| v.phase().clone()), Phase::NoRepos);
    assert!(h.is_laid_out("helm-charts-no-repos"));
    assert!(h.is_laid_out("helm-charts-open-repos"));
    assert!(!h.is_laid_out("helm-chart-cards"));
    h.click("helm-charts-add:bitnami");
    assert!(matches!(
        h.bus.sent().as_slice(),
        [Command::HelmRepoAdd { name, .. }] if name == "bitnami"
    ));
    h.click("helm-charts-open-repos");
    assert_eq!(h.bus.sent().last(), Some(&Command::HelmShowRepos));
}

#[gpui::test]
fn without_helm_the_page_explains_and_offers_nothing(cx: &mut TestAppContext) {
    let mut h = Harness::open_without_helm(cx);
    assert_eq!(h.read(|v| v.phase().clone()), Phase::NoHelm);
    assert!(h.is_laid_out("helm-charts-guidance"));
    assert!(!h.is_laid_out("helm-chart-cards"));
    assert!(!h.is_laid_out("helm-charts-search"));
    assert!(
        !h.is_laid_out("helm-charts-repos"),
        "no repositories button either"
    );
    assert!(
        h.fake
            .recorded_calls()
            .iter()
            .all(|c| matches!(c, HelmCall::Repos)),
        "only the repository list was tried"
    );
    assert!(h.bus.sent().is_empty());
}

#[gpui::test]
fn a_repository_change_reads_the_catalog_again(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"], nginx_and_redis());
    h.fake
        .script()
        .repos
        .push_ok(vec![repo("bitnami"), repo("jetstack")]);
    h.fake
        .script()
        .search
        .push_ok(vec![hit("jetstack", "cert-manager", "1.0.0", "1", "Certs")]);
    // The repository service bumps the cue on a successful add; the page follows it.
    h.fake
        .script()
        .repos
        .push_ok(vec![repo("bitnami"), repo("jetstack")]);
    h.vcx.update(|_, cx| {
        let repos = h.repos.clone();
        cx.spawn(async move |_| {
            repos
                .add(oxikube_domain::helm::RepoAddRequest::new(
                    "jetstack",
                    "https://j",
                ))
                .await
                .ok();
        })
        .detach();
    });
    h.settle();
    h.read(|v| {
        assert_eq!(v.phase(), &Phase::Ready { repos: 2 });
        assert_eq!(v.cards()[0].name, "cert-manager");
    });
}

#[gpui::test]
fn two_thousand_cards_build_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let hits: Vec<_> = (0..2_000)
        .map(|i| hit("bitnami", &format!("chart-{i:04}"), "1.0.0", "1", "A chart"))
        .collect();
    let mut h = Harness::open(cx, &["bitnami"], hits);
    assert_eq!(h.read(|v| v.cards().len()), 2_000);
    assert!(h.is_laid_out("helm-chart-card:bitnami/chart-0000"));
    assert!(
        !h.is_laid_out("helm-chart-card:bitnami/chart-1999"),
        "a row off screen is not built"
    );
}
