//! The guard in front of the real `helm` (E16-S12, scenario 3): a read-only cluster refuses
//! install, upgrade, rollback and uninstall for the UI and an agent alike (each denial audited,
//! the release untouched), and a wrong release name never uninstalls.

use oxikube_app::command_bus::{DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::{InstallError, InstallFlow, InstallSpec};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{ChartRef, ReleaseRef, ReleaseStatus};
use oxikube_ports::HelmPort as _;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::helm::{Driver, HelmBin, write_chart};

use super::{WHO, World, cli_config, rollback, suffix, uninstall};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_only_blocks_every_helm_mutation_and_uninstall_needs_the_name() {
    let Some(helm) = HelmBin::find() else {
        return;
    };
    let Some(w) = World::connect(cli_config(&helm)).await else {
        return;
    };
    let ns = TestNamespace::create(w.context()).expect("namespace");
    let namespace = ns.name();
    let charts = tempfile::tempdir().unwrap();
    let chart = write_chart(charts.path(), "0.1.0", false);
    let chart_text = chart.display().to_string();
    let name = format!("ro-{}", suffix(namespace));
    helm.run(
        w.context(),
        namespace,
        Driver::Secret,
        &["install", &name, &chart_text],
    )
    .expect("helm install");
    let release = ReleaseRef::new(w.cluster.clone(), namespace, &name);
    let native = w.native();
    let before = native.history(&release).await.unwrap().len();

    // --- a wrong name is refused; the release stays --------------------------------------------
    let command = uninstall(&w.cluster, namespace, &name, false);
    let Ok(Outcome::NeedsConfirmation(asked)) = w.bus.dispatch(command.clone(), w.ui()).await
    else {
        panic!("uninstall asks first");
    };
    let wrong = w
        .bus
        .dispatch(
            command,
            w.ui()
                .with_confirmation(Confirmation::typed(asked.token, "not-it")),
        )
        .await;
    assert!(
        matches!(wrong, Err(DispatchError::Confirmation(_))),
        "{wrong:?}"
    );
    assert_eq!(
        native.history(&release).await.unwrap()[0].status,
        ReleaseStatus::Deployed
    );

    // --- read-only: all four refused for every initiator ---------------------------------------
    w.manager.set_read_only(&w.cluster, true).unwrap();
    let fresh = format!("new-{}", suffix(namespace));
    let commands = [
        Command::HelmInstall {
            cluster: w.cluster.clone(),
            release: fresh.clone(),
            namespace: namespace.into(),
            create_namespace: false,
            repo: None,
            chart: chart_text.clone(),
            version: None,
            values: Default::default(),
            wait: false,
            timeout_secs: None,
        },
        Command::HelmUpgrade {
            cluster: w.cluster.clone(),
            namespace: namespace.into(),
            release: name.clone(),
            repo: None,
            chart: chart_text.clone(),
            version: None,
            values: Default::default(),
            reuse_values: false,
            wait: false,
            timeout_secs: None,
        },
        rollback(&w.cluster, namespace, &name, 1),
        uninstall(&w.cluster, namespace, &name, false),
    ];
    let denied_before = denied(&w);
    for initiator in [Initiator::Ui, Initiator::Agent] {
        let ctx = DispatchContext::new(initiator, WHO).with_cluster(w.cluster.clone());
        for command in &commands {
            let id = command.id();
            let refused = w.bus.dispatch(command.clone(), ctx.clone()).await;
            assert!(
                matches!(refused, Err(DispatchError::ReadOnly { .. })),
                "{id} by {initiator:?}: {refused:?}"
            );
        }
    }
    assert_eq!(denied(&w) - denied_before, 8, "every refusal audited");
    // The install tab's preview is refused too (nothing rendered or sent).
    let flow = InstallFlow::new(w.bus.clone(), w.manager.clone(), w.progress.clone(), WHO);
    let spec = InstallSpec::new(
        w.cluster.clone(),
        &fresh,
        namespace,
        ChartRef::new(&chart_text, ""),
    );
    let preview = flow.preview(spec).await;
    assert!(
        matches!(preview, Err(InstallError::ReadOnly { .. })),
        "{preview:?}"
    );

    // The cluster is as it was.
    let after = native.history(&release).await.unwrap();
    assert_eq!(after.len(), before);
    assert_eq!(after[0].status, ReleaseStatus::Deployed);
    let fresh_ref = ReleaseRef::new(w.cluster.clone(), namespace, &fresh);
    assert!(
        native.history(&fresh_ref).await.is_err(),
        "nothing installed"
    );
    w.manager.set_read_only(&w.cluster, false).unwrap();
}

fn denied(w: &World) -> usize {
    w.audit()
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Denied)
        .count()
}
