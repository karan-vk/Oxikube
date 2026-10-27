//! ServiceAccount kubeconfig generation (E17-S09) against the real adapter: the guarded
//! `serviceaccount::GenerateKubeconfig` requests a real `TokenRequest` on a kind cluster, and the
//! kubeconfig it builds authenticates as that ServiceAccount (and is limited to what its Role
//! allows). No UI.
//!
//! The server address and CA come from the kind context's own kubeconfig entry, handed to the
//! handler through a scripted source port: `oxikube_kube`'s `cluster_endpoint` (the real source)
//! is tested against files in that crate.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use k8s_openapi::api::core::v1::Pod;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config};
use oxikube_app::command_bus::{CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::Confirmation;
use oxikube_app::sa_kubeconfig::{KubeconfigVault, register_commands};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_ports::ClusterEndpoint;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry, read_only_pod_viewer};

/// The kind context's server address and CA, as its kubeconfig entry says.
fn endpoint_of(kind: &Kind) -> ClusterEndpoint {
    let cluster = kind.kubeconfig.clusters[0]
        .cluster
        .as_ref()
        .expect("the kind cluster entry");
    let pem = match (
        cluster.certificate_authority_data.as_deref(),
        cluster.certificate_authority.as_deref(),
    ) {
        (Some(data), _) => {
            String::from_utf8(STANDARD.decode(data).expect("base64 CA")).expect("PEM")
        }
        (None, Some(file)) => std::fs::read_to_string(file).expect("the CA file"),
        (None, None) => panic!("the kind context has no CA"),
    };
    ClusterEndpoint {
        server: cluster.server.clone().expect("a server"),
        certificate_authority_pem: Some(pem),
        insecure_skip_tls_verify: false,
        tls_server_name: cluster.tls_server_name.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_generated_kubeconfig_authenticates_as_the_service_account_and_nothing_more() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let admin = kind.admin_client().await;
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    // The account, a Role that reads pods in this namespace only, and its binding.
    drop(read_only_pod_viewer(&admin, ns.name(), "viewer").await);

    let entry = catalog_entry(&kind.context);
    let sources = Arc::new(FakeClusterSourcePort::new().with_contexts([entry.clone()]));
    sources
        .script()
        .cluster_endpoint
        .push_ok(endpoint_of(&kind));
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
    let state = Arc::new(FakeStatePort::new());
    let vault = KubeconfigVault::new();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::sa_kubeconfig", |r| {
            register_commands(r, manager.clone(), sources.clone(), vault.clone())
        })
        .expect("register the command");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
    );
    let account = ResourceRef::namespaced(
        entry.cluster.clone(),
        Gvk::new("", "v1", "ServiceAccount"),
        ns.name(),
        "viewer",
    );
    let command = Command::ServiceAccountGenerateKubeconfig {
        target: account.clone(),
        expiration_seconds: Some(600),
        audiences: Vec::new(),
        allow_legacy_token: false,
    };

    // The guard asks first; nothing is held until it is answered.
    let ctx = DispatchContext::new(Initiator::Ui, "tester");
    let Ok(Outcome::NeedsConfirmation(request)) = bus.dispatch(command.clone(), ctx.clone()).await
    else {
        panic!("expected a confirmation request");
    };
    assert!(vault.is_empty());
    let Ok(Outcome::Completed(output)) = bus
        .dispatch(
            command,
            ctx.with_confirmation(Confirmation::simple(request.token)),
        )
        .await
    else {
        panic!("the confirmed command completes");
    };
    let data = output.data.expect("structured result");
    assert_eq!(data["source"], "token_request");
    assert!(
        data["expires_at"].is_string(),
        "the server said when it expires"
    );

    let held = vault.get(&account).expect("the kubeconfig is in the vault");

    assert!(
        !held.preview.contains(
            held.text
                .expose()
                .lines()
                .find_map(|line| line.trim().strip_prefix("token: "))
                .expect("a token line")
        )
    );
    let config = Kubeconfig::from_yaml(held.text.expose()).expect("valid kubeconfig YAML");
    assert_eq!(
        config.current_context.as_deref(),
        Some(&*format!("viewer@{}", kind.context))
    );
    let config = Config::from_custom_kubeconfig(config, &KubeConfigOptions::default())
        .await
        .expect("a usable client config");
    let client = Client::try_from(config).expect("a client");

    // It authenticates as the account and may read pods in the account's namespace...
    Api::<Pod>::namespaced(client.clone(), ns.name())
        .list(&Default::default())
        .await
        .expect("the Role lets the account list pods in its namespace");
    // ...and only there: the API server names the identity it saw.
    let error = Api::<Pod>::namespaced(client, "kube-system")
        .list(&Default::default())
        .await
        .expect_err("the Role does not reach kube-system");
    let message = error.to_string();
    assert!(
        message.contains(&format!("system:serviceaccount:{}:viewer", ns.name())),
        "the token is the account's: {message}"
    );

    let audit = state.audit_log();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(&*audit[0].cmd, "serviceaccount::GenerateKubeconfig");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(
        audit[0].detail.as_deref(),
        Some("expiration=600s audiences=0 legacy_allowed=false")
    );
    assert!(
        !format!("{audit:?}").contains(
            held.text
                .expose()
                .lines()
                .find_map(|line| line.trim().strip_prefix("token: "))
                .unwrap()
        )
    );
}
