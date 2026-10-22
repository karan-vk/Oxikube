//! No-binary mode on kind (E16-S12, scenario 4): `helm.path` names a file that does not exist.
//! The CLI reports nothing usable, so only native reads are offered; every mutation through the
//! guard fails without touching the release; the native reader still lists the release and reads
//! its history and values. Needs no `helm` at all: the release records are written as Helm stores
//! them.

use oxikube_domain::helm::{HelmCapabilities, ReleaseRef, ReleaseStatus};
use oxikube_domain::session::NamespaceSelection;
use oxikube_helm::HelmCliConfig;
use oxikube_ports::HelmPort as _;
use oxikube_testkit::integration::helm::{CHART_NAME, Driver, release_record};
use oxikube_testkit::integration::{TestNamespace, events};

use super::{World, rollback, suffix, uninstall};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_helm_binary_leaves_native_reads_only() {
    let missing = tempfile::tempdir().unwrap();
    let config = HelmCliConfig {
        path: Some(missing.path().join("helm")),
        ..HelmCliConfig::default()
    };
    let Some(w) = World::connect(config).await else {
        return;
    };
    let ns = TestNamespace::create(w.context()).expect("namespace");
    let namespace = ns.name();
    let name = format!("nobin-{}", suffix(namespace));
    for revision in [1, 2] {
        events::create(
            w.context(),
            namespace,
            &release_record(Driver::Secret, namespace, &name, revision),
        )
        .expect("create a release record");
    }

    // The CLI offers nothing; the native reader offers reads only: the views hide every
    // mutating action and the charts catalog.
    let cli = w.cli.detect().await;
    assert!(
        !cli.cli_available && !cli.can_mutate() && !cli.can_search_repos,
        "{cli:?}"
    );
    let native = w.native();
    assert_eq!(native.capabilities(), HelmCapabilities::native_only());
    assert!(!native.capabilities().can_mutate());

    // Reads work.
    let listed = native
        .list_releases(&w.cluster, &NamespaceSelection::single(namespace))
        .await
        .expect("native list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].revision,
            listed[0].status,
            listed[0].chart.name.as_str()
        ),
        (2, ReleaseStatus::Deployed, CHART_NAME)
    );
    let release = ReleaseRef::new(w.cluster.clone(), namespace, &name);
    assert_eq!(native.history(&release).await.unwrap().len(), 2);
    assert!(native.values(&release, None, true).await.is_ok());

    // A mutation that reaches the handler (confirmed) fails: there is no binary to run.
    for command in [
        rollback(&w.cluster, namespace, &name, 1),
        uninstall(&w.cluster, namespace, &name, false),
    ] {
        let id = command.id();
        let failed = w.confirm_and_run(command, w.ui()).await;
        assert!(failed.is_err(), "{id} without a binary: {failed:?}");
    }
    let history = native.history(&release).await.unwrap();
    assert_eq!(history.len(), 2, "the release is untouched");
    assert_eq!(history[0].status, ReleaseStatus::Deployed);
}
