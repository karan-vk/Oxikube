//! The Helm feature end to end on kind (E16-S12): the real `helm` CLI adapter (`HelmCli`), the
//! native release reader (`NativeHelmReader` over the session's resource port) and the app's
//! install/upgrade flows and rollback/uninstall commands through the `CommandBus` and
//! `MutationGuard`, wired the way the binary wires them. No UI.
//!
//! | File | Scenario |
//! |---|---|
//! | `lifecycle` | a release made by the real `helm` (both storage drivers) listed natively; install (with values), upgrade (new values and chart version), rollback to revision 1, uninstall with and without `--keep-history` through the flows and the guard, the native view checked after each step |
//! | `guard` | a read-only cluster blocks all four for every initiator (audited, the release untouched); rollback asks a simple confirmation, uninstall the release name |
//! | `no_binary` | `helm.path` naming a missing file: native-only capabilities, every mutation refused, reads still work |
//! | `scale` | 500 release Secrets (written as Helm stores them, not installed) listed natively within the budget |
//!
//! The CLI scenarios skip with a notice when no `helm` >= 3.10 is found (`OXIKUBE_TEST_HELM`
//! names another binary). Charts are local (`oxikube_testkit::integration::helm::write_chart`):
//! ConfigMaps only, nothing pulled or scheduled. Every object lives in the test's own
//! `oxi-test-<rand>` namespace; release names carry its random suffix.

mod guard;
mod lifecycle;
mod no_binary;
mod scale;

use std::sync::Arc;

use k8s_openapi::api::core::v1::ConfigMap;
use kube::Api;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, DispatchError, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard, ReleaseProgress, UpgradeRequests};
use oxikube_domain::audit::AuditRecord;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_helm::{HelmCli, HelmCliConfig, HelmTarget, NativeHelmReader};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::HelmPort;
use oxikube_testkit::integration::helm::HelmBin;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// The local user the flows dispatch as.
pub const WHO: &str = "tester";

/// One connected kind session and the Helm commands registered over one `HelmCli`.
pub struct World {
    pub kind: Kind,
    pub admin: kube::Client,
    pub manager: ClusterSessionManager,
    pub bus: CommandBus,
    pub state: Arc<FakeStatePort>,
    pub cluster: ClusterId,
    pub progress: ReleaseProgress,
    pub cli: Arc<HelmCli>,
}

impl World {
    /// Connects to the kind cluster (`None` when `OXIKUBE_TEST_CONTEXT` is unset) with a
    /// `HelmCli` of `config` behind `helm::Install`, `helm::Upgrade`, `helm::Rollback` and
    /// `helm::Uninstall`, as `bins/oxikube` registers them.
    pub async fn connect(config: HelmCliConfig) -> Option<Self> {
        let kind = Kind::from_env().await?;
        let admin = kind.admin_client().await;
        let entry = catalog_entry(&kind.context);
        let cluster = entry.cluster.clone();
        let connector = KubeConnector::new(
            kind.kubeconfig.clone(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        );
        let manager = ClusterSessionManager::with_config(
            Arc::new(connector),
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&cluster).await.expect("connect");

        // `helm` reads the developer's (or CI's) default kubeconfig, which defines the context.
        let (target_cluster, context) = (cluster.clone(), kind.context.clone());
        let cli = Arc::new(HelmCli::new(
            config,
            Arc::new(move |c: &ClusterId| {
                (*c == target_cluster).then(|| HelmTarget::new(context.clone(), None))
            }),
        ));
        let progress = ReleaseProgress::new();
        let mut registry = CommandRegistry::new();
        let port: Arc<dyn HelmPort> = cli.clone();
        let (install_port, upgrade_port) = (port.clone(), port.clone());
        let (install_progress, upgrade_progress) = (progress.clone(), progress.clone());
        registry
            .install("oxikube_app::helm_install", move |r| {
                oxikube_app::helm_install::register_commands(r, install_port, install_progress)
            })
            .expect("register helm::Install");
        registry
            .install("oxikube_app::helm_upgrade", move |r| {
                oxikube_app::helm_upgrade::register_commands(
                    r,
                    upgrade_port,
                    upgrade_progress,
                    UpgradeRequests::new(),
                )
            })
            .expect("register helm::Upgrade");
        registry
            .install("oxikube_app::helm_ops", move |r| {
                oxikube_app::helm_ops::register_commands(r, port)
            })
            .expect("register helm::Rollback, helm::Uninstall");
        let state = Arc::new(FakeStatePort::new());
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        Some(Self {
            kind,
            admin,
            manager,
            bus,
            state,
            cluster,
            progress,
            cli,
        })
    }

    /// The kubectl context name.
    pub fn context(&self) -> &str {
        self.kind.context.as_str()
    }

    /// A native reader over the session's resource port, as the Releases view builds one. It
    /// runs no binary: it reads the release Secrets and ConfigMaps.
    pub fn native(&self) -> NativeHelmReader {
        let session = self.manager.get(&self.cluster).expect("session");
        let resources = session.resources().expect("connected");
        NativeHelmReader::new(self.cluster.clone(), resources)
    }

    /// A UI dispatch context on the cluster.
    pub fn ui(&self) -> DispatchContext {
        DispatchContext::new(oxikube_domain::audit::Initiator::Ui, WHO)
            .with_cluster(self.cluster.clone())
    }

    /// Dispatches `command`, answers the guard's confirmation as the dialog would (the release
    /// name typed when asked for), and returns the final outcome.
    pub async fn confirm_and_run(
        &self,
        command: Command,
        ctx: DispatchContext,
    ) -> Result<Outcome, DispatchError> {
        match self.bus.dispatch(command.clone(), ctx.clone()).await? {
            Outcome::NeedsConfirmation(request) => {
                let answer = match &request.expected_name {
                    Some(name) => Confirmation::typed(request.token, name.clone()),
                    None => Confirmation::simple(request.token),
                };
                self.bus
                    .dispatch(command, ctx.with_confirmation(answer))
                    .await
            }
            done => Ok(done),
        }
    }

    /// Every audit record so far.
    pub fn audit(&self) -> Vec<AuditRecord> {
        self.state.audit_log()
    }

    /// The `data` of ConfigMap `name` in `namespace`, `None` when it does not exist.
    pub async fn config_map(
        &self,
        namespace: &str,
        name: &str,
    ) -> Option<std::collections::BTreeMap<String, String>> {
        Api::<ConfigMap>::namespaced(self.admin.clone(), namespace)
            .get_opt(name)
            .await
            .expect("get config map")
            .map(|cm| cm.data.unwrap_or_default())
    }
}

/// A `HelmCliConfig` that runs the binary under test.
pub fn cli_config(helm: &HelmBin) -> HelmCliConfig {
    HelmCliConfig {
        path: Some(helm.path().to_path_buf()),
        ..HelmCliConfig::default()
    }
}

/// The last 8 characters of `namespace` (`oxi-test-<8 hex>`): unique per run, for release names.
pub fn suffix(namespace: &str) -> &str {
    &namespace[namespace.len() - 8..]
}

/// `helm::Rollback` of `release` in `namespace` to `revision`.
pub fn rollback(cluster: &ClusterId, namespace: &str, release: &str, revision: u32) -> Command {
    Command::HelmRollback {
        cluster: cluster.clone(),
        namespace: namespace.into(),
        release: release.into(),
        revision,
    }
}

/// `helm::Uninstall` of `release` in `namespace`.
pub fn uninstall(
    cluster: &ClusterId,
    namespace: &str,
    release: &str,
    keep_history: bool,
) -> Command {
    Command::HelmUninstall {
        cluster: cluster.clone(),
        namespace: namespace.into(),
        release: release.into(),
        keep_history,
    }
}
