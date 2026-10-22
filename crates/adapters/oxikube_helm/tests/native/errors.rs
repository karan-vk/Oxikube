//! Failures: missing releases, undecodable records, the size cap, RBAC, and the operations
//! the native reader does not have.

use std::io::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::helm::{
    ChartRef, HelmCapabilities, HelmPort, InstallRequest, RollbackRequest, SearchQuery, ShowKind,
    UninstallRequest, UpgradeRequest,
};
use oxikube_ports::resource::ListPage;

use crate::support::{Driver, cluster, object_with_data, reader, release_ref, secret};

fn assert_kind<T: std::fmt::Debug>(result: Result<T, OxiError>, kind: ErrorKind) -> OxiError {
    match result {
        Err(e) => {
            assert_eq!(e.kind(), kind, "{e}");
            e
        }
        Ok(v) => panic!("expected a {kind:?} error, got {v:?}"),
    }
}

#[tokio::test]
async fn missing_releases_and_revisions_are_not_found() {
    let (reader, _) = reader([secret("a", "web", 1, "deployed")]);
    let e = assert_kind(
        reader.history(&release_ref("a", "nope")).await,
        ErrorKind::NotFound,
    );
    assert!(e.message().contains("a/nope"), "{e}");
    assert_kind(
        reader.history(&release_ref("b", "web")).await,
        ErrorKind::NotFound,
    );
    let e = assert_kind(
        reader.manifest(&release_ref("a", "web"), Some(7)).await,
        ErrorKind::NotFound,
    );
    assert!(e.message().contains("revision 7"), "{e}");
    // A name that could not be a release never reaches a label selector.
    assert_kind(
        reader
            .values(&release_ref("a", "web,owner"), None, false)
            .await,
        ErrorKind::NotFound,
    );
}

#[tokio::test]
async fn undecodable_records_are_parse_errors_not_panics() {
    let gz_truncated = {
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(br#"{"name":"t","info":{"status":"deployed"}}"#)
            .unwrap();
        let bytes = gz.finish().unwrap();
        STANDARD.encode(&bytes[..bytes.len() - 8])
    };
    let truncated_json = {
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(br#"{"name":"j","config":{"password":"hunter2""#)
            .unwrap();
        STANDARD.encode(gz.finish().unwrap())
    };
    let cases = [
        ("bad-base64", "!!not base64!!".to_owned()),
        ("not-gzip-not-json", STANDARD.encode("plain text")),
        ("truncated-gzip", gz_truncated),
        ("truncated-json", truncated_json),
    ];
    let objects = cases
        .iter()
        .map(|(name, data)| object_with_data(Driver::ConfigMap, "a", name, 1, "deployed", data));
    let (reader, _) = reader(objects);
    for (name, _) in &cases {
        let e = assert_kind(
            reader.manifest(&release_ref("a", name), None).await,
            ErrorKind::Validation,
        );
        assert!(
            e.message().contains("could not parse Helm data"),
            "{name}: {e}"
        );
        assert!(!e.message().contains("hunter2"), "payload leaked: {e}");
    }
    // The list still shows all four, from labels.
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(releases.len(), cases.len());
}

#[tokio::test]
async fn a_record_inflating_past_the_cap_is_refused() {
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    let chunk = vec![b' '; 1024 * 1024];
    gz.write_all(b"{\"name\":\"bomb\",\"manifest\":\"").unwrap();
    for _ in 0..51 {
        gz.write_all(&chunk).unwrap();
    }
    gz.write_all(b"\"}").unwrap();
    let data = STANDARD.encode(gz.finish().unwrap());
    let (reader, _) = reader([object_with_data(
        Driver::ConfigMap,
        "a",
        "bomb",
        1,
        "deployed",
        &data,
    )]);
    let e = assert_kind(
        reader.manifest(&release_ref("a", "bomb"), None).await,
        ErrorKind::Validation,
    );
    assert!(e.message().contains("inflates to more than 50 MiB"), "{e}");
}

#[tokio::test]
async fn a_secret_of_another_type_is_not_decoded() {
    let mut fake_type = secret("a", "web", 1, "deployed");
    fake_type.edit_json(|json| json["type"] = "Opaque".into());
    let (reader, _) = reader([fake_type]);
    let e = assert_kind(
        reader.manifest(&release_ref("a", "web"), None).await,
        ErrorKind::Validation,
    );
    assert!(e.message().contains("helm.sh/release.v1"), "{e}");
}

#[tokio::test]
async fn an_unreadable_driver_is_skipped_but_both_failing_is_an_error() {
    let (reader, fake) = reader([secret("a", "web", 1, "deployed")]);
    // Secrets are listed first, then ConfigMaps: forbid the ConfigMap list.
    fake.script()
        .list_metadata
        .push_ok(ListPage::complete(vec![
            secret("a", "web", 1, "deployed").meta,
        ]))
        .push_err(OxiError::forbidden("configmaps is forbidden"));
    let releases = reader
        .list_releases(&cluster(), &NamespaceSelection::All)
        .await
        .unwrap();
    assert_eq!(releases.len(), 1);

    fake.script()
        .list_metadata
        .push_err(OxiError::forbidden("secrets is forbidden"))
        .push_err(OxiError::forbidden("configmaps is forbidden"));
    assert_kind(
        reader
            .list_releases(&cluster(), &NamespaceSelection::All)
            .await,
        ErrorKind::Forbidden,
    );
}

#[tokio::test]
async fn another_clusters_requests_are_refused() {
    let (reader, _) = reader([secret("a", "web", 1, "deployed")]);
    let other = ClusterId::new("kubeconfig-test", &ContextName::new("prod"));
    assert_kind(
        reader.list_releases(&other, &NamespaceSelection::All).await,
        ErrorKind::Validation,
    );
}

#[tokio::test]
async fn cli_operations_are_unsupported_and_capabilities_say_native_only() {
    let (reader, _) = reader([]);
    assert_eq!(reader.capabilities(), HelmCapabilities::native_only());
    let web = release_ref("a", "web");
    let chart = ChartRef::new("nginx", "").in_repo("bitnami");
    let install = InstallRequest::new(cluster(), "web", "a", chart.clone());
    let unsupported = ErrorKind::Unsupported;
    assert_kind(reader.repos().await, unsupported);
    assert_kind(reader.search(&SearchQuery::default()).await, unsupported);
    assert_kind(reader.show(&chart, ShowKind::Readme).await, unsupported);
    assert_kind(reader.template(&install).await, unsupported);
    assert!(matches!(reader.install(&install).await, Err(e) if e.kind() == unsupported));
    let upgrade = UpgradeRequest::new(web.clone(), chart);
    assert!(matches!(reader.upgrade(&upgrade).await, Err(e) if e.kind() == unsupported));
    assert_kind(
        reader.rollback(&RollbackRequest::new(web.clone(), 1)).await,
        unsupported,
    );
    assert_kind(
        reader.uninstall(&UninstallRequest::new(web)).await,
        unsupported,
    );
}
