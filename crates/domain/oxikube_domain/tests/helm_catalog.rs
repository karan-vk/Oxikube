//! Chart catalog grouping and semver-aware version ordering (E16-S08).

use std::cmp::Ordering;

use oxikube_domain::helm::{ChartHit, ChartRef, compare_versions, group_hits};

fn hit(repo: &str, name: &str, version: &str, app: Option<&str>, desc: &str) -> ChartHit {
    ChartHit {
        chart: ChartRef::new(name, version).in_repo(repo),
        app_version: app.map(str::to_owned),
        description: desc.to_owned(),
    }
}

#[test]
fn versions_sort_by_semver_not_by_text() {
    let charts = group_hits(vec![
        hit("b", "nginx", "9.0.0", Some("1.0"), "old"),
        hit("b", "nginx", "15.10.0", Some("1.25"), "new"),
        hit("b", "nginx", "15.9.1", Some("1.24"), "mid"),
        hit("b", "nginx", "15.10.0-rc.1", None, "rc"),
    ]);
    assert_eq!(charts.len(), 1);
    let versions: Vec<_> = charts[0]
        .versions
        .iter()
        .map(|v| v.version.as_str())
        .collect();
    assert_eq!(versions, ["15.10.0", "15.10.0-rc.1", "15.9.1", "9.0.0"]);
    assert_eq!(charts[0].latest().description, "new");
    assert_eq!(charts[0].latest().app_version.as_deref(), Some("1.25"));
}

#[test]
fn charts_group_per_repo_and_name_and_sort_by_name() {
    let charts = group_hits(vec![
        hit("bitnami", "redis", "1.0.0", None, ""),
        hit("other", "nginx", "1.0.0", None, ""),
        hit("bitnami", "nginx", "2.0.0", None, ""),
        hit("bitnami", "nginx", "2.0.0", None, "duplicate"),
    ]);
    let refs: Vec<_> = charts.iter().map(|c| c.reference()).collect();
    assert_eq!(refs, ["bitnami/nginx", "other/nginx", "bitnami/redis"]);
    assert_eq!(
        charts[0].versions.len(),
        1,
        "a repeated version is kept once"
    );
    assert_eq!(charts[0].chart_ref(None).version, "2.0.0");
    assert_eq!(
        charts[0].chart_ref(Some("1.0.0")).repo.as_deref(),
        Some("bitnami")
    );
}

#[test]
fn empty_input_gives_no_charts() {
    assert!(group_hits(Vec::new()).is_empty());
}

#[test]
fn comparison_handles_prereleases_prefixes_and_junk() {
    assert_eq!(compare_versions("1.0.0", "1.0.0-rc.1"), Ordering::Greater);
    assert_eq!(compare_versions("v1.2", "1.2.0"), Ordering::Equal);
    assert_eq!(compare_versions("1.0.0+build5", "1.0.0"), Ordering::Equal);
    assert_eq!(
        compare_versions("1.0.0-alpha.2", "1.0.0-alpha.10"),
        Ordering::Less
    );
    assert_eq!(
        compare_versions("1.0.0-alpha", "1.0.0-alpha.1"),
        Ordering::Less
    );
    assert_eq!(compare_versions("1.0.0-1", "1.0.0-a"), Ordering::Less);
    assert_eq!(compare_versions("latest", "0.0.1"), Ordering::Less);
    assert_eq!(compare_versions("a", "b"), Ordering::Less);
}
