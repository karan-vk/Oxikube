//! The `helm` CLI adapter against a real `helm` and the kind cluster (`cargo xtask kind-up`):
//! `OXIKUBE_TEST_CONTEXT=kind-oxikube cargo test -p oxikube_helm --features integration --test
//! kind_helm_cli`. Skips when the variable is unset or no `helm` >= 3.10 is installed.
//! `OXIKUBE_TEST_HELM=/path/to/helm` tests another binary (for example Helm 3.10 and 4.x).
//!
//! The chart is written by the test: one ConfigMap and a NOTES.txt, so nothing is pulled or
//! scheduled on the shared cluster. Everything lives in a namespace of its own, deleted at the
//! end; the release name is unique to that namespace. The full Helm lifecycle against public
//! charts is E16-S12.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use oxikube_domain::ErrorKind;
use oxikube_domain::helm::{
    ChartRef, InstallRequest, ReleaseRef, ReleaseStatus, RollbackRequest, UninstallRequest,
    UpgradeRequest, Values,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::NamespaceSelection;
use oxikube_helm::{HelmCli, HelmCliConfig, HelmTarget};
use oxikube_ports::{HelmOutput, HelmPort as _};
use oxikube_testkit::integration::{TestNamespace, test_context};

const TOKEN: &str = "s3cr3t-token-for-the-redaction-check";

fn write_chart(dir: &Path) {
    std::fs::create_dir_all(dir.join("templates"))
        .expect("the chart directory is creatable in a temp dir");
    std::fs::write(
        dir.join("Chart.yaml"),
        "apiVersion: v2\nname: oxi-demo\nversion: 0.1.0\nappVersion: \"1.0\"\n",
    )
    .expect("Chart.yaml is writable in the temp chart dir");
    std::fs::write(dir.join("values.yaml"), "greeting: hello\ntoken: \"\"\n")
        .expect("values.yaml is writable in the temp chart dir");
    std::fs::write(
        dir.join("templates/configmap.yaml"),
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {{ .Release.Name }}\n\
         data:\n  greeting: {{ .Values.greeting | quote }}\n",
    )
    .expect("the configmap template is writable in the temp chart dir");
    std::fs::write(
        dir.join("templates/NOTES.txt"),
        "Installed {{ .Release.Name }}.\n",
    )
    .expect("NOTES.txt is writable in the temp chart dir");
}

async fn drain(output: HelmOutput) -> Vec<String> {
    let items: Vec<_> = output.collect().await;
    items
        .into_iter()
        .map(|item| item.expect("the operation succeeds").text)
        .collect()
}

fn values(yaml: &str) -> Values {
    Values::from_yaml(yaml).expect("the test values literal is valid YAML")
}

#[tokio::test]
async fn a_release_lifecycle_through_the_real_helm() {
    let Some(context) = test_context() else {
        return;
    };
    let ctx = ContextName::new(context.as_str());
    let cluster = ClusterId::new("kind-test", &ctx);
    let cli = HelmCli::new(
        HelmCliConfig {
            path: std::env::var_os("OXIKUBE_TEST_HELM").map(std::path::PathBuf::from),
            ..HelmCliConfig::default()
        },
        Arc::new(move |_: &ClusterId| Some(HelmTarget::new(ctx.clone(), None))),
    );
    let caps = cli.detect().await;
    if !caps.cli_available {
        eprintln!("skipping: no helm >= 3.10 found ({:?})", caps.cli_version);
        return;
    }
    let namespace = TestNamespace::create(&context).expect("test namespace");
    let ns = namespace.name().to_owned();
    let chart_dir = tempfile::tempdir().unwrap();
    write_chart(chart_dir.path());
    let chart = ChartRef::new(chart_dir.path().display().to_string(), "");
    let name = format!("rel-{}", &ns[ns.len() - 8..]);
    let release = ReleaseRef::new(cluster.clone(), ns.clone(), name.clone());

    let mut install = InstallRequest::new(cluster.clone(), name.clone(), ns.clone(), chart.clone());
    install.values = values(&format!("greeting: hi\ntoken: {TOKEN}\n"));
    install.timeout = Duration::from_secs(60);
    let rendered = cli.template(&install).await.unwrap();
    assert!(rendered.contains("greeting: \"hi\""), "{rendered}");

    let lines = drain(cli.install(&install).await.unwrap()).await;
    assert!(
        lines.iter().any(|l| l.contains("STATUS: deployed")),
        "{lines:?}"
    );
    assert!(lines.iter().all(|l| !l.contains(TOKEN)));

    let listed = cli
        .list_releases(&cluster, &NamespaceSelection::single(&ns))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].release, release);
    assert_eq!(
        (listed[0].revision, listed[0].status),
        (1, ReleaseStatus::Deployed)
    );
    assert_eq!(listed[0].chart, ChartRef::new("oxi-demo", "0.1.0"));
    assert_eq!(listed[0].app_version.as_deref(), Some("1.0"));

    let user = cli.values(&release, None, false).await.unwrap();
    assert_eq!(user.get("greeting"), Some(&serde_json::json!("hi")));
    let notes = cli.notes(&release, None).await.unwrap().unwrap();
    assert!(notes.contains(&format!("Installed {name}.")), "{notes}");
    assert!(
        cli.manifest(&release, None)
            .await
            .unwrap()
            .contains("kind: ConfigMap")
    );

    let mut upgrade = UpgradeRequest::new(release.clone(), chart.clone());
    upgrade.values = values("greeting: bonjour\n");
    upgrade.timeout = Duration::from_secs(60);
    drain(cli.upgrade(&upgrade).await.unwrap()).await;
    let history = cli.history(&release).await.unwrap();
    let numbers: Vec<u32> = history.iter().map(|r| r.revision).collect();
    assert_eq!(numbers, [2, 1]);

    let mut rollback = RollbackRequest::new(release.clone(), 1);
    rollback.timeout = Duration::from_secs(60);
    cli.rollback(&rollback).await.unwrap();
    let user = cli.values(&release, None, false).await.unwrap();
    assert_eq!(user.get("greeting"), Some(&serde_json::json!("hi")));

    let again = cli.install(&install).await.unwrap();
    let items: Vec<_> = again.collect().await;
    let error = items.last().unwrap().as_ref().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Conflict, "{error:?}");
    assert!(!error.message().contains(TOKEN));

    let mut uninstall = UninstallRequest::new(release.clone());
    uninstall.timeout = Duration::from_secs(60);
    cli.uninstall(&uninstall).await.unwrap();
    let gone = cli.history(&release).await.unwrap_err();
    assert_eq!(gone.kind(), ErrorKind::NotFound);
}
