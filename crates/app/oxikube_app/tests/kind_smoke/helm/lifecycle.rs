//! A release's whole life on kind (E16-S12, scenarios 1, 2 and 5): releases made by the real
//! `helm` with either storage driver read natively; then install, upgrade, rollback and both
//! kinds of uninstall through the install/upgrade flows and the guarded commands (the path the
//! tabs and dialogs take), the native view and the cluster's objects checked after every step.

use futures::StreamExt as _;
use oxikube_app::command_bus::Outcome;
use oxikube_app::{InstallFlow, InstallSpec, UpgradeFlow, UpgradeSpec};
use oxikube_domain::ErrorKind;
use oxikube_domain::helm::{ChartRef, ReleaseRef, ReleaseStatus, ValuesYaml};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::session::NamespaceSelection;
use oxikube_helm::NativeHelmReader;
use oxikube_ports::HelmPort as _;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::helm::{CHART_NAME, Driver, HelmBin, write_chart};
use serde_json::json;
use std::sync::Arc;

use super::{WHO, World, cli_config, rollback, suffix, uninstall};

const TOKEN: &str = "s3cr3t-token-for-the-audit-check";

/// `(revision, status)` of every revision of `release`, newest first.
async fn history(native: &NativeHelmReader, release: &ReleaseRef) -> Vec<(u32, ReleaseStatus)> {
    native
        .history(release)
        .await
        .expect("history")
        .iter()
        .map(|r| (r.revision, r.status))
        .collect()
}

fn local(chart: &std::path::Path) -> ChartRef {
    ChartRef::new(chart.display().to_string(), "")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn helm_releases_from_the_cli_and_through_the_guard() {
    let Some(helm) = HelmBin::find() else {
        return;
    };
    let Some(w) = World::connect(cli_config(&helm)).await else {
        return;
    };
    let ns = TestNamespace::create(w.context()).expect("namespace");
    let (namespace, sfx) = (ns.name(), suffix(ns.name()));
    let charts = tempfile::tempdir().unwrap();
    let v1 = write_chart(&charts.path().join("v1"), "0.1.0", false);
    let v2 = write_chart(&charts.path().join("v2"), "0.2.0", true);
    let native = w.native();
    eprintln!("helm {} in {namespace}", helm.version());

    // --- 1 and 5: made by the real CLI, both drivers, read natively ----------------------------
    let (by_cli, by_cli_cm) = (format!("cli-{sfx}"), format!("cm.cli-{sfx}"));
    let v1_path = v1.to_str().unwrap();
    helm.run(
        w.context(),
        namespace,
        Driver::Secret,
        &["install", &by_cli, v1_path, "--set", "greeting=from-cli"],
    )
    .expect("helm install (secret driver)");
    helm.run(
        w.context(),
        namespace,
        Driver::ConfigMap,
        &["install", &by_cli_cm, v1_path],
    )
    .expect("helm install (configmap driver)");
    let listed = native
        .list_releases(&w.cluster, &NamespaceSelection::single(namespace))
        .await
        .expect("native list");
    let rows: Vec<_> = listed
        .iter()
        .map(|r| {
            (
                r.release.name.as_str(),
                r.revision,
                r.status,
                r.chart.to_string(),
            )
        })
        .collect();
    let chart_v1 = format!("{CHART_NAME}-0.1.0");
    assert_eq!(
        rows,
        vec![
            (
                by_cli.as_str(),
                1,
                ReleaseStatus::Deployed,
                chart_v1.clone()
            ),
            (
                by_cli_cm.as_str(),
                1,
                ReleaseStatus::Deployed,
                chart_v1.clone()
            ),
        ]
    );
    let cli_ref = ReleaseRef::new(w.cluster.clone(), namespace, &by_cli);
    let user = native.values(&cli_ref, None, false).await.unwrap();
    assert_eq!(user.get("greeting"), Some(&json!("from-cli")));
    let cm_ref = ReleaseRef::new(w.cluster.clone(), namespace, &by_cli_cm);
    let all = native.values(&cm_ref, None, true).await.unwrap();
    assert_eq!(all.get("greeting"), Some(&json!("hello")), "chart defaults");

    // --- 2a: install with values: preview, confirm (risk low), streamed output -----------------
    let name = format!("app-{sfx}");
    let release = ReleaseRef::new(w.cluster.clone(), namespace, &name);
    let install = InstallFlow::new(w.bus.clone(), w.manager.clone(), w.progress.clone(), WHO);
    let mut spec = InstallSpec::new(w.cluster.clone(), &name, namespace, local(&v1));
    spec.values = ValuesYaml::new(format!("greeting: hi\ntoken: {TOKEN}\n"));
    let preview = install.preview(spec).await.expect("install preview");
    assert!(preview.server_error.is_none(), "{:?}", preview.server_error);
    assert_eq!(preview.summary.kinds.get("ConfigMap"), Some(&1));
    assert!(
        native.history(&release).await.is_err(),
        "the preview installs nothing"
    );
    let asked = install.ask(&preview).await.expect("ask");
    assert_eq!(
        (asked.tier, asked.risk),
        (ConfirmTier::Simple, Some(Risk::Low))
    );
    let started = install.start(&preview, &asked);
    let (lines, result) = futures::join!(started.lines.collect::<Vec<_>>(), started.run);
    result.expect("installed");
    assert!(
        lines.iter().any(|l| l.text.contains("STATUS: deployed")),
        "{lines:?}"
    );
    assert_eq!(
        history(&native, &release).await,
        [(1, ReleaseStatus::Deployed)]
    );
    let user = native.values(&release, None, false).await.unwrap();
    assert_eq!(user.get("greeting"), Some(&json!("hi")));
    let cfg = w
        .config_map(namespace, &format!("{name}-cfg"))
        .await
        .expect("cfg made");
    assert_eq!(cfg["greeting"], "hi");

    // --- 2b: upgrade to chart 0.2.0 with changed values: preview names the added object --------
    let upgrade = UpgradeFlow::new(w.bus.clone(), Arc::new(w.native()), w.progress.clone(), WHO);
    let current = upgrade.current(&release).await.expect("current release");
    assert_eq!(
        (current.revision, current.chart.to_string()),
        (1, chart_v1.clone())
    );
    let mut spec = UpgradeSpec::new(release.clone(), local(&v2));
    spec.values = ValuesYaml::new("greeting: bonjour\n");
    let preview = upgrade.preview(spec).await.expect("upgrade preview");
    assert!(preview.can_upgrade(), "{:?}", preview.server_error);
    let added: Vec<_> = preview
        .changes
        .added
        .iter()
        .map(|k| k.name.as_str())
        .collect();
    assert_eq!(added, [format!("{name}-extra")]);
    let asked = upgrade.ask(&preview).await.expect("ask");
    assert_eq!(
        (asked.tier, asked.risk),
        (ConfirmTier::Simple, Some(Risk::Medium))
    );
    let started = upgrade.start(&preview, &asked);
    let (_, result) = futures::join!(started.lines.collect::<Vec<_>>(), started.run);
    result.expect("upgraded");
    assert_eq!(
        history(&native, &release).await,
        [(2, ReleaseStatus::Deployed), (1, ReleaseStatus::Superseded)]
    );
    let latest = &native.history(&release).await.unwrap()[0];
    assert_eq!(latest.chart.to_string(), format!("{CHART_NAME}-0.2.0"));
    let cfg = w
        .config_map(namespace, &format!("{name}-cfg"))
        .await
        .unwrap();
    assert_eq!(
        (cfg["greeting"].as_str(), cfg["chart"].as_str()),
        ("bonjour", "0.2.0")
    );
    assert!(
        w.config_map(namespace, &format!("{name}-extra"))
            .await
            .is_some()
    );

    // --- 2c: rollback to revision 1 (simple confirmation, risk medium) --------------------------
    let command = rollback(&w.cluster, namespace, &name, 1);
    let Ok(Outcome::NeedsConfirmation(asked)) = w.bus.dispatch(command.clone(), w.ui()).await
    else {
        panic!("rollback asks first");
    };
    assert_eq!(
        (asked.tier, asked.risk),
        (ConfirmTier::Simple, Some(Risk::Medium))
    );
    w.bus.decline(asked.token).await.expect("decline");
    w.confirm_and_run(command, w.ui())
        .await
        .expect("rolled back");
    assert_eq!(
        history(&native, &release).await,
        [
            (3, ReleaseStatus::Deployed),
            (2, ReleaseStatus::Superseded),
            (1, ReleaseStatus::Superseded)
        ]
    );
    let latest = &native.history(&release).await.unwrap()[0];
    assert_eq!(latest.chart.to_string(), chart_v1);
    let user = native.values(&release, None, false).await.unwrap();
    assert_eq!(
        user.get("greeting"),
        Some(&json!("hi")),
        "revision 1's values"
    );
    assert_eq!(
        w.config_map(namespace, &format!("{name}-cfg"))
            .await
            .unwrap()["greeting"],
        "hi"
    );
    crate::eventually("the 0.2.0-only object to go", String::new, || async {
        w.config_map(namespace, &format!("{name}-extra"))
            .await
            .is_none()
    })
    .await;

    // --- 2d: uninstall keeping the history (the name typed, risk high) -------------------------
    let command = uninstall(&w.cluster, namespace, &name, true);
    let Ok(Outcome::NeedsConfirmation(asked)) = w.bus.dispatch(command.clone(), w.ui()).await
    else {
        panic!("uninstall asks first");
    };
    assert_eq!(
        (asked.tier, asked.risk),
        (ConfirmTier::TypeName, Some(Risk::High))
    );
    assert_eq!(asked.expected_name.as_deref(), Some(name.as_str()));
    w.bus.decline(asked.token).await.expect("decline");
    w.confirm_and_run(command, w.ui())
        .await
        .expect("uninstalled");
    let kept = history(&native, &release).await;
    assert_eq!(kept.len(), 3, "{kept:?}");
    assert_eq!(kept[0], (3, ReleaseStatus::Uninstalled));
    crate::eventually("the release's objects to go", String::new, || async {
        w.config_map(namespace, &format!("{name}-cfg"))
            .await
            .is_none()
    })
    .await;

    // --- 2e: uninstall without history: the CLI-made release disappears from the native view ---
    w.confirm_and_run(uninstall(&w.cluster, namespace, &by_cli, false), w.ui())
        .await
        .expect("uninstalled");
    let gone = native.history(&cli_ref).await.unwrap_err();
    assert_eq!(gone.kind(), ErrorKind::NotFound, "{gone:?}");
    crate::eventually("the CLI release's objects to go", String::new, || async {
        w.config_map(namespace, &format!("{by_cli}-cfg"))
            .await
            .is_none()
    })
    .await;
    let names: Vec<_> = native
        .list_releases(&w.cluster, &NamespaceSelection::single(namespace))
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.release.name)
        .collect();
    assert!(!names.contains(&by_cli), "{names:?}");

    // Every attempt audited, never with the values.
    let audit = w.audit();
    assert!(!audit.is_empty());
    assert!(
        audit.iter().all(|r| !format!("{r:?}").contains(TOKEN)),
        "no audit record carries a value"
    );
}
