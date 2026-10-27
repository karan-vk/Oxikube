//! `serviceaccount::GenerateKubeconfig` over `oxikube_testkit` fakes, through the real
//! `CommandBus` and `MutationGuard` (see [`crate::testing::Harness`]).
//!
//! * `build`: the kubeconfig text (embedded CA, masked preview, comments, round trip)
//! * `token`: the `TokenRequest` body and response, the lifetime check
//! * `handler`: the command (guard, audit, no token outside the vault, dry run)
//! * `legacy`: clusters without `TokenRequest`

mod build;
mod handler;
mod legacy;
mod token;

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::ClusterEndpoint;
use oxikube_testkit::FakeClusterSourcePort;
use serde_json::{Value, json};

use super::{KubeconfigVault, register_commands};
use crate::testing::{Harness, cluster_context, id};

pub(super) const CLUSTER: &str = "a";

/// A secret the tests look for in every place it must not appear.
pub(super) const TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.sa-token-payload.signature";

pub(super) const CA: &str = "-----BEGIN CERTIFICATE-----\nMIIBfake\n-----END CERTIFICATE-----\n";

pub(super) struct Fixture {
    pub h: Harness,
    pub vault: KubeconfigVault,
}

/// A harness with the command registered, connected writable to cluster `a`, with the account
/// `shop/deployer` in it.
pub(super) fn fixture() -> Fixture {
    let vault = KubeconfigVault::new();
    let sources = Arc::new(FakeClusterSourcePort::new().with_contexts([cluster_context(CLUSTER)]));
    sources.script().cluster_endpoint.push_ok(ClusterEndpoint {
        server: "https://a.example:6443".into(),
        certificate_authority_pem: Some(CA.into()),
        insecure_skip_tls_verify: false,
        tls_server_name: None,
    });
    let h = {
        let (vault, sources) = (vault.clone(), sources);
        Harness::with_extra(move |registry, _, manager| {
            register_commands(registry, manager.clone(), sources, vault)
        })
    };
    h.connect(CLUSTER, false);
    h.resources(CLUSTER)
        .insert(account_object("shop", "deployer"));
    Fixture { h, vault }
}

pub(super) fn account_object(namespace: &str, name: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ServiceAccount",
        "metadata": {"name": name, "namespace": namespace},
    }))
    .expect("a ServiceAccount")
}

pub(super) fn account() -> ResourceRef {
    ResourceRef::namespaced(
        id(CLUSTER),
        Gvk::new("", "v1", "ServiceAccount"),
        "shop",
        "deployer",
    )
}

pub(super) fn generate(expiration_seconds: Option<u32>, allow_legacy_token: bool) -> Command {
    Command::ServiceAccountGenerateKubeconfig {
        target: account(),
        expiration_seconds,
        audiences: Vec::new(),
        allow_legacy_token,
    }
}

/// What the API server answers a `TokenRequest` with.
pub(super) fn issued(token: &str, expires: &str) -> Value {
    json!({
        "apiVersion": "authentication.k8s.io/v1", "kind": "TokenRequest",
        "status": {"token": token, "expirationTimestamp": expires},
    })
}
