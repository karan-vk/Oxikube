//! The RBAC epic end to end against the real adapter, no UI (E17-S10): a restricted
//! ServiceAccount's kubeconfig, generated the way the app generates it, gates what the app offers;
//! the app's subject index answers what `kubectl auth can-i --list` answers; a binding made
//! through the bus and the guard grants what it says.
//!
//! The scenario is `oxikube_testkit::fixtures::rbac::RbacScenario`, applied with
//! `oxikube_testkit::integration::rbac::AppliedScenario`: two `oxi-test-<rand>-*` namespaces,
//! `home` and `other`, and in `home` the account `limited`, which may read pods there and nothing
//! else of its own. Both namespaces (and so everything the tests create) are deleted when a test
//! ends, also when it fails. The same objects drive the offline `access::tests::scenario`.
//!
//! | Module | Epic E17 "done when" it proves |
//! |---|---|
//! | `gating` | (1) the sections the account may not list are hidden, the rest shown; (5) the accessible-namespaces fallback when the namespace list is forbidden |
//! | `matrix` | (2) the index's rules and per-verb matrix equal `kubectl auth can-i --list` for the account, before and after a binding; (3) the binding is made through `binding::Create` and the guard and works (the aggregated `view`) |
//! | `token` | (4) the generated kubeconfig works with `kubectl` and expires when asked |
//!
//! The binding dialog's own model (`BindingDraft`) building that binding, and the app's sidebar
//! drawing what `gating` computes, are `bins/oxikube/tests/kind_rbac.rs`.
//!
//! Tokens are secrets: they stay in the vault's `SecretText` and in memory; the one kubeconfig
//! written to disk (`token`) is in a temp dir, owner-only, deleted when the test ends.

mod gating;
mod matrix;
mod token;

use std::sync::Arc;

use kube::config::Kubeconfig;
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::sa_kubeconfig::{GeneratedKubeconfig, KubeconfigVault};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard, binding_ops, sa_kubeconfig};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ContextName, Gvk, ResourceRef};
use oxikube_domain::session::NamespaceSelection;
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::{ClusterContext, ClusterEndpoint};
use oxikube_testkit::fixtures::rbac::ACCOUNT;
use oxikube_testkit::integration::rbac::AppliedScenario;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};
use crate::sa_kubeconfig::endpoint_of;

/// The kind admin's session, with a bus that runs the RBAC epic's mutations through the guard.
pub struct Admin {
    pub entry: ClusterContext,
    pub bus: CommandBus,
    pub vault: KubeconfigVault,
    sources: Arc<FakeClusterSourcePort>,
    endpoint: ClusterEndpoint,
}

impl Admin {
    /// Connects the kind context and registers `serviceaccount::GenerateKubeconfig` (the server
    /// address and CA scripted from the kind context's own kubeconfig entry, as in
    /// `sa_kubeconfig`) and the `binding::*` commands.
    pub async fn connect(kind: &Kind) -> Self {
        let entry = catalog_entry(&kind.context);
        let sources = Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()]));
        let manager = ClusterSessionManager::with_config(
            Arc::new(KubeConnector::new(
                kind.kubeconfig.clone(),
                PoolConfig::default(),
                ConnectorConfig::default(),
            )),
            sources.clone(),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        let vault = KubeconfigVault::new();
        let mut registry = CommandRegistry::new();
        registry
            .install("oxikube_app::sa_kubeconfig", |r| {
                sa_kubeconfig::register_commands(r, manager.clone(), sources.clone(), vault.clone())
            })
            .expect("register the kubeconfig command");
        registry
            .install("oxikube_app::binding_ops", binding_ops::register_commands)
            .expect("register the binding commands");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(
                manager.clone(),
                Arc::new(FakeStatePort::new()),
                Arc::new(TokioClock),
            ),
        );
        Self {
            entry,
            bus,
            vault,
            sources,
            endpoint: endpoint_of(kind),
        }
    }

    /// Dispatches `command` as the UI does: asked first, then confirmed.
    pub async fn run_confirmed(&self, command: Command) -> Outcome {
        let ctx =
            DispatchContext::new(Initiator::Ui, "tester").with_cluster(self.entry.cluster.clone());
        let Ok(Outcome::NeedsConfirmation(request)) =
            self.bus.dispatch(command.clone(), ctx.clone()).await
        else {
            panic!("expected a confirmation request");
        };
        self.bus
            .dispatch(
                command,
                ctx.with_confirmation(Confirmation::simple(request.token)),
            )
            .await
            .expect("the confirmed command runs")
    }

    /// A kubeconfig for the scenario's account, generated through the guarded
    /// `serviceaccount::GenerateKubeconfig` (`expiration_seconds` `None` is the default lifetime).
    pub async fn generate_kubeconfig(
        &self,
        applied: &AppliedScenario,
        expiration_seconds: Option<u32>,
    ) -> GeneratedKubeconfig {
        let account = ResourceRef::namespaced(
            self.entry.cluster.clone(),
            Gvk::new("", "v1", "ServiceAccount"),
            applied.scenario().home.as_str(),
            ACCOUNT,
        );
        self.sources
            .script()
            .cluster_endpoint
            .push_ok(self.endpoint.clone());
        let outcome = self
            .run_confirmed(Command::ServiceAccountGenerateKubeconfig {
                target: account.clone(),
                expiration_seconds,
                audiences: Vec::new(),
                allow_legacy_token: false,
            })
            .await;
        assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");
        self.vault.get(&account).expect("the kubeconfig is held")
    }
}

/// The account's own session, from the generated kubeconfig alone, watching `selection`.
pub struct Restricted {
    pub manager: ClusterSessionManager,
    pub entry: ClusterContext,
}

impl Restricted {
    /// Connects the generated kubeconfig's current context.
    pub async fn connect(generated: &GeneratedKubeconfig, selection: NamespaceSelection) -> Self {
        let kubeconfig =
            Kubeconfig::from_yaml(generated.text.expose()).expect("valid kubeconfig YAML");
        let context = ContextName::from(
            kubeconfig
                .current_context
                .clone()
                .expect("the generated kubeconfig names its context"),
        );
        let entry = catalog_entry(&context);
        let manager = ClusterSessionManager::with_config(
            Arc::new(KubeConnector::new(
                kubeconfig,
                PoolConfig::default(),
                ConnectorConfig::default(),
            )),
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        manager.open(
            &entry,
            SessionOptions {
                namespace_selection: selection,
                ..SessionOptions::default()
            },
        );
        manager
            .connect(&entry.cluster)
            .await
            .expect("the account connects with its kubeconfig");
        Self { manager, entry }
    }
}
