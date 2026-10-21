//! The `-o json` parsers over output recorded from Helm 3.10.3 and 4.3.1 against kind.

use oxikube_domain::helm::{ChartRef, HelmVersion, ReleaseStatus};
use oxikube_domain::ids::{ClusterId, ContextName};

use super::{VERSIONS, fixture};
use crate::cli::detect::parse_version;
use crate::cli::parse;

fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::new("kind-oxikube"))
}

#[test]
fn version_output_of_each_major() {
    assert_eq!(
        parse_version(&fixture("helm-3.10", "version.out")),
        Some(HelmVersion::new(3, 10, 3))
    );
    assert_eq!(
        parse_version(&fixture("helm-4.3", "version.out")),
        Some(HelmVersion::new(4, 3, 1))
    );
    assert_eq!(
        parse_version("Client: v2.16.1+gbbdfe5e"),
        Some(HelmVersion::new(2, 16, 1))
    );
    assert_eq!(parse_version("garbage"), None);
    assert_eq!(parse_version(""), None);
}

#[test]
fn list_gives_latest_revision_per_release() {
    for version in VERSIONS {
        let releases = parse::releases(&cluster(), &fixture(version, "list.out")).unwrap();
        assert_eq!(releases.len(), 2, "{version}");
        let a1 = &releases[0];
        assert_eq!(a1.release.name, "a1");
        assert!(a1.release.namespace.starts_with("helm-rec2-"));
        assert_eq!(a1.release.cluster, cluster());
        assert_eq!(a1.status, ReleaseStatus::Uninstalled);
        assert_eq!(a1.revision, 1);
        assert_eq!(a1.chart, ChartRef::new("demo", "0.1.0"));
        assert_eq!(a1.app_version.as_deref(), Some("1.16.0"));
        let b2 = &releases[1];
        assert_eq!((b2.revision, b2.status), (2, ReleaseStatus::Deployed));
        // Go's `time.String` (`2026-10-21 15:58:52.421201 +0900 JST`), read with its offset.
        assert!(b2.updated > a1.updated, "{version}");
        assert!(
            b2.updated.to_string().starts_with("2026-10-21T06:58:"),
            "{}",
            b2.updated
        );
        assert!(b2.notes.is_none() && b2.values_user.is_empty());
    }
}

#[test]
fn history_is_newest_first() {
    for version in VERSIONS {
        let revisions = parse::history(&fixture(version, "history.out")).unwrap();
        let numbers: Vec<u32> = revisions.iter().map(|r| r.revision).collect();
        assert_eq!(numbers, [2, 1], "{version}");
        assert_eq!(revisions[0].status, ReleaseStatus::Deployed);
        assert_eq!(revisions[0].description, "Upgrade complete");
        assert_eq!(revisions[1].status, ReleaseStatus::Superseded);
        assert_eq!(revisions[1].description, "Install complete");
        assert_eq!(revisions[1].chart, ChartRef::new("demo", "0.1.0"));
        assert!(revisions[1].updated < revisions[0].updated);
    }
}

#[test]
fn user_and_computed_values() {
    for version in VERSIONS {
        let user = parse::values(&fixture(version, "values.out")).unwrap();
        assert_eq!(user.get("replicaCount"), Some(&serde_json::json!(3)));
        assert_eq!(user.get("image.tag"), Some(&serde_json::json!("1.25")));
        let all = parse::values(&fixture(version, "values_all.out")).unwrap();
        assert_eq!(
            all.get("image.repository"),
            Some(&serde_json::json!("nginx"))
        );
        assert_eq!(all.get("service.port"), Some(&serde_json::json!(80)));
    }
    assert!(
        parse::values("null\n").unwrap().is_empty(),
        "no user values"
    );
}

#[test]
fn notes_drop_the_header() {
    for version in VERSIONS {
        let notes = parse::notes(&fixture(version, "notes.out")).unwrap();
        assert!(
            notes.starts_with("1. Get the application URL"),
            "{version}: {notes}"
        );
    }
    assert_eq!(parse::notes("NOTES:\n\n"), None);
    assert_eq!(parse::notes(""), None);
}

#[test]
fn repo_list_and_empty() {
    for version in VERSIONS {
        let repos = parse::repos(&fixture(version, "repo_list.out")).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "podinfo");
        assert_eq!(repos[0].url, "https://stefanprodan.github.io/podinfo");
        assert!(
            parse::repos(&fixture(version, "repo_list_empty.out"))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn search_splits_repo_and_chart() {
    for version in VERSIONS {
        let hits = parse::hits(&fixture(version, "search.out")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].chart,
            ChartRef::new("podinfo", "6.15.0").in_repo("podinfo")
        );
        assert_eq!(hits[0].app_version.as_deref(), Some("6.15.0"));
        assert_eq!(hits[0].description, "Podinfo Helm chart for Kubernetes");
        let versions = parse::hits(&fixture(version, "search_versions.out")).unwrap();
        assert_eq!(versions.len(), 5);
        assert!(versions.iter().all(|h| h.chart.name == "podinfo"));
        assert!(
            parse::hits(&fixture(version, "search_none.out"))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn chart_column_splits_at_the_version() {
    assert_eq!(parse::chart("demo-0.1.0"), ChartRef::new("demo", "0.1.0"));
    assert_eq!(
        parse::chart("kube-prometheus-stack-51.2.0"),
        ChartRef::new("kube-prometheus-stack", "51.2.0")
    );
    assert_eq!(
        parse::chart("my-app2-1.0.0-rc.1"),
        ChartRef::new("my-app2", "1.0.0-rc.1")
    );
    assert_eq!(
        parse::chart("cert-manager-v1.14.0"),
        ChartRef::new("cert-manager", "v1.14.0")
    );
    assert_eq!(
        parse::chart("vault-0.28.0"),
        ChartRef::new("vault", "0.28.0")
    );
    assert_eq!(parse::chart("plain"), ChartRef::new("plain", ""));
    assert_eq!(
        parse::chart("web-3d-viewer-1.0.0"),
        ChartRef::new("web-3d-viewer", "1.0.0")
    );
    assert_eq!(
        parse::chart("auth-2fa-v2-v1.2.3"),
        ChartRef::new("auth-2fa-v2", "v1.2.3")
    );
    assert_eq!(
        parse::chart("app-2024.x-0.1.0"),
        ChartRef::new("app-2024.x", "0.1.0")
    );
}

#[test]
fn timestamps_of_both_formats() {
    let go = parse::timestamp("2026-10-21 15:58:01.274296 +0900 JST");
    let rfc = parse::timestamp("2026-10-21T15:58:01.274296+09:00");
    assert_eq!(go, rfc);
    assert_eq!(go.to_string(), "2026-10-21T06:58:01.274296Z");
    assert_eq!(parse::timestamp("yesterday"), jiff::Timestamp::UNIX_EPOCH);
}

#[test]
fn malformed_json_is_a_parse_error() {
    let error = parse::releases(&cluster(), "Error: not json").unwrap_err();
    assert!(matches!(error, oxikube_domain::helm::HelmError::Parse(_)));
    // A newer Helm adding fields or statuses does not break the list.
    let out = r#"[{"name":"x","namespace":"n","revision":"7","status":"brand-new","chart":"c-1.0.0","extra":true}]"#;
    let releases = parse::releases(&cluster(), out).unwrap();
    assert_eq!(
        (releases[0].revision, releases[0].status),
        (7, ReleaseStatus::Unknown)
    );
    assert_eq!(releases[0].app_version, None);
}
